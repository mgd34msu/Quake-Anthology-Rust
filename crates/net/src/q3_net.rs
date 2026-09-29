//! Quake III channels, reliable commands, snapshots, downloads, rcon,
//! admission, and client/server connections.
//!
//! Donor provenance: `src/network/q3/{netchan,reliable,connectionless,
//! transport,demo,game-state,configstrings,pure,server-message,
//! client-message,parse-entities,snapshot-history,snapshot-store,
//! client-server-command,download,rcon,admission,client,server,clock,
//! state-delta,state/entity,state/player}.ts` (ports of id Software's
//! `qcommon`, `client`, and `server` sources).
//!
//! The donor's promise-based owners become synchronous bindings over
//! [`DatagramTransport`]; every state transition keeps the donor's order and
//! limits. Bitstream coding reuses [`crate::q3`], adaptive Huffman reuses
//! [`crate::huffman`], and command/info/numeric helpers reuse `qa-core`.

use std::collections::{HashMap, HashSet};

use qa_core::cmd::{Dialect, TextMode, source_command_text, tokenize_command};
use qa_core::cvar::{InfoOptions, InfoTarget, set_info_value};
use qa_core::identity::{ClientId, SeatId};
use qa_core::numeric::native_atoi;
use thiserror::Error;

use crate::common::endpoint::{NetworkAddress, same_address};
use crate::common::hash::{md4_block_checksum, md4_block_checksum_key};
use crate::huffman::{HuffmanError, compress_adaptive, decompress_adaptive};
use crate::q3::{
    MessageMode, Q3MsgError, Q3MsgReader, Q3MsgWriter, WireUserCommand, MAX_MESSAGE_LENGTH,
    read_delta_user_command, write_delta_user_command,
};

/// Error for Quake III netcode.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3NetError {
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] Q3MsgError),
    /// Underlying Huffman failure.
    #[error("{0}")]
    Huffman(#[from] HuffmanError),
    /// Underlying command-text failure.
    #[error("{0}")]
    Cmd(#[from] qa_core::cmd::CmdError),
    /// Underlying cvar failure.
    #[error("{0}")]
    Cvar(#[from] qa_core::cvar::CvarError),
    /// Underlying numeric failure.
    #[error("{0}")]
    Numeric(#[from] qa_core::numeric::NumericError),
    /// Underlying transport failure.
    #[error("{0}")]
    Transport(#[from] crate::common::transport::TransportError),
    /// Socket failure, with the display text of the I/O error.
    #[error("socket failure: {0}")]
    Io(String),
    /// Dropped connection, with the donor message.
    #[error("{kind}: {message}")]
    Drop {
        /// Drop kind (`drop`, `fatal`, `server-disconnect`).
        kind: &'static str,
        /// Message.
        message: String,
    },
    /// Client command overflow (`ReliableOverflowError`).
    #[error("Client command overflow")]
    ReliableOverflow {
        /// Current sequence.
        sequence: i32,
        /// Acknowledged sequence.
        acknowledge: i32,
    },
    /// Invalid client opcode.
    #[error("Invalid client opcode {opcode}")]
    ClientOpcode {
        /// Opcode value.
        opcode: i32,
    },
    /// Invalid server opcode.
    #[error("Invalid server opcode {opcode}")]
    ServerOpcode {
        /// Opcode value.
        opcode: i32,
    },
    /// Invalid gamestate opcode.
    #[error("Invalid gamestate opcode {opcode}")]
    GamestateOpcode {
        /// Opcode value.
        opcode: i32,
    },
    /// Bad delta entity number.
    #[error("Bad delta entity number {number}")]
    BadDeltaEntity {
        /// Number value.
        number: i32,
    },
    /// Invalid entity last field.
    #[error("Invalid entity last field {last}")]
    EntityLastField {
        /// Last value.
        last: i32,
    },
    /// Invalid player last field.
    #[error("Invalid player last field {last}")]
    PlayerLastField {
        /// Last value.
        last: i32,
    },
    /// Invalid user command count.
    #[error("Invalid user command count {count}")]
    ClientCommandCount {
        /// Count value.
        count: i32,
    },
    /// Protocol-level failure.
    #[error("{0}")]
    Protocol(&'static str),
    /// Range failure.
    #[error("{0}")]
    Range(&'static str),
}

impl Q3NetError {
    /// Build a drop error.
    fn drop(kind: &'static str, message: impl Into<String>) -> Self {
        Self::Drop { kind, message: message.into() }
    }
}

/// Source message diagnostics state (`SourceMessageState`).
pub struct SourceMessageState {
    print: Box<dyn FnMut(&str) + Send + Sync>,
    oldsize: Option<i32>,
    newsize: Option<i32>,
    overflows: Option<i32>,
}

impl SourceMessageState {
    /// Build state over a print sink.
    pub fn new(print: impl FnMut(&str) + Send + Sync + 'static) -> Self {
        Self { print: Box::new(print), oldsize: Some(0), newsize: Some(0), overflows: Some(0) }
    }

    /// Print a diagnostic.
    pub fn print(&mut self, text: &str) {
        (self.print)(text);
    }

    /// Add old-size bits with null-on-undefined arithmetic.
    pub fn add_oldsize(&mut self, bits: i32) {
        self.oldsize = add_counter(self.oldsize, bits);
    }

    /// Add new-size bytes with null-on-undefined arithmetic.
    pub fn add_newsize(&mut self, bytes: i32) {
        self.newsize = add_counter(self.newsize, bytes);
    }

    /// Check a written value against its width.
    pub fn check_overflow(&mut self, value: i32, bits: i32) {
        if bits == 32 {
            return;
        }
        if bits < 0 || bits == 31 {
            self.overflows = None;
            return;
        }
        if value > (1i32 << bits) - 1 || value < 0 {
            self.overflows = add_counter(self.overflows, 1);
        }
    }

    /// Old-size counter.
    #[must_use]
    pub fn oldsize(&self) -> Option<i32> {
        self.oldsize
    }

    /// New-size counter.
    #[must_use]
    pub fn newsize(&self) -> Option<i32> {
        self.newsize
    }

    /// Overflow counter.
    #[must_use]
    pub fn overflows(&self) -> Option<i32> {
        self.overflows
    }
}

fn add_counter(value: Option<i32>, increment: i32) -> Option<i32> {
    value?.checked_add(increment)
}

/// Product tag (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Product {
    /// Base Quake III.
    Base,
    /// Team Arena.
    MissionPack,
}

/// Trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q3Trajectory {
    /// Trajectory type tag (raw mod values preserved).
    pub trajectory_type: i32,
    /// Time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base.
    pub base: [f32; 3],
    /// Delta.
    pub delta: [f32; 3],
}

/// Entity state (`EntityStateFields`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q3EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Q3Trajectory,
    /// Angle trajectory.
    pub apos: Q3Trajectory,
    /// Time.
    pub time: i32,
    /// Second time.
    pub time2: i32,
    /// Origin.
    pub origin: [f32; 3],
    /// Second origin.
    pub origin2: [f32; 3],
    /// Angles.
    pub angles: [f32; 3],
    /// Second angles.
    pub angles2: [f32; 3],
    /// Other entity number.
    pub other_entity_num: i32,
    /// Second other entity number.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Second model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic value.
    pub generic1: i32,
}

/// Player state slots (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PlayerSlots {
    values: [i32; 16],
}

impl Q3PlayerSlots {
    /// Zeroed slots.
    #[must_use]
    pub fn new() -> Self {
        Self { values: [0; 16] }
    }

    /// Read a slot.
    pub fn get(&self, index: usize) -> Result<i32, Q3NetError> {
        self.values
            .get(index)
            .copied()
            .ok_or(Q3NetError::Range("Player state slot outside 16"))
    }

    /// Write a slot.
    pub fn set(&mut self, index: usize, value: i32) -> Result<(), Q3NetError> {
        let Some(slot) = self.values.get_mut(index) else {
            return Err(Q3NetError::Range("Player state slot outside 16"));
        };
        *slot = value;
        Ok(())
    }
}

impl Default for Q3PlayerSlots {
    fn default() -> Self {
        Self::new()
    }
}

/// Player state (`PlayerStateFields`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PlayerState {
    /// Product tag.
    pub product: Q3Product,
    /// Command time.
    pub command_time: i32,
    /// Movement type.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Origin.
    pub origin: [f32; 3],
    /// Velocity.
    pub velocity: [f32; 3],
    /// Weapon timer.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: [f32; 3],
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Legs timer.
    pub legs_timer: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso timer.
    pub torso_timer: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: [f32; 3],
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Events.
    pub events: [i32; 2],
    /// Event parameters.
    pub event_parms: [i32; 2],
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: [f32; 3],
    /// View height.
    pub viewheight: i32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: Q3PlayerSlots,
    /// Persistant data.
    pub persistant: Q3PlayerSlots,
    /// Powerups.
    pub powerups: Q3PlayerSlots,
    /// Ammo.
    pub ammo: Q3PlayerSlots,
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping (not on the wire).
    pub ping: i32,
    /// Pmove frame count (not on the wire).
    pub pmove_framecount: i32,
    /// Jump-pad frame (not on the wire).
    pub jumppad_frame: i32,
    /// Entity event sequence (not on the wire).
    pub entity_event_sequence: i32,
}

impl Q3PlayerState {
    /// Zeroed player state for a product.
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: 0,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: [0.0; 3],
            velocity: [0.0; 3],
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: [0.0; 3],
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: [0.0; 3],
            e_flags: 0,
            event_sequence: 0,
            events: [0; 2],
            event_parms: [0; 2],
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: 0,
            weapon_state: 0,
            viewangles: [0.0; 3],
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: Q3PlayerSlots::new(),
            persistant: Q3PlayerSlots::new(),
            powerups: Q3PlayerSlots::new(),
            ammo: Q3PlayerSlots::new(),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }
}

/// Removed-entity marker (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;
/// Entity number bits.
pub const ENTITY_NUMBER_BITS: i32 = 10;
/// Entity space.
pub const MAX_Q3_ENTITIES: i32 = 1 << ENTITY_NUMBER_BITS;
/// Float-as-int bits.
const FLOAT_INT_BITS: i32 = 13;
/// Float-as-int bias.
const FLOAT_INT_BIAS: i32 = 4096;

/// One delta-coded entity field.
#[derive(Debug, Clone, Copy)]
struct EntityField {
    name: &'static str,
    bits: i32,
    kind: EntityFieldKind,
}

#[derive(Debug, Clone, Copy)]
enum EntityFieldKind {
    PosTime,
    PosBase(usize),
    PosDelta(usize),
    AposBase(usize),
    AposDelta(usize),
    Event,
    Angles2(usize),
    EType,
    TorsoAnim,
    EventParm,
    LegsAnim,
    GroundEntityNum,
    PosType,
    EFlags,
    OtherEntityNum,
    Weapon,
    ClientNum,
    Angles(usize),
    PosDuration,
    AposType,
    Origin(usize),
    Solid,
    Powerups,
    Modelindex,
    OtherEntityNum2,
    LoopSound,
    Generic1,
    Origin2(usize),
    Modelindex2,
    Time,
    AposTime,
    AposDuration,
    Time2,
    ConstantLight,
    Frame,
}

const ENTITY_FIELDS: &[EntityField] = &[
    field("pos.trTime", 32, EntityFieldKind::PosTime),
    field("pos.trBase[0]", 0, EntityFieldKind::PosBase(0)),
    field("pos.trBase[1]", 0, EntityFieldKind::PosBase(1)),
    field("pos.trDelta[0]", 0, EntityFieldKind::PosDelta(0)),
    field("pos.trDelta[1]", 0, EntityFieldKind::PosDelta(1)),
    field("pos.trBase[2]", 0, EntityFieldKind::PosBase(2)),
    field("apos.trBase[1]", 0, EntityFieldKind::AposBase(1)),
    field("pos.trDelta[2]", 0, EntityFieldKind::PosDelta(2)),
    field("apos.trBase[0]", 0, EntityFieldKind::AposBase(0)),
    field("event", 10, EntityFieldKind::Event),
    field("angles2[1]", 0, EntityFieldKind::Angles2(1)),
    field("eType", 8, EntityFieldKind::EType),
    field("torsoAnim", 8, EntityFieldKind::TorsoAnim),
    field("eventParm", 8, EntityFieldKind::EventParm),
    field("legsAnim", 8, EntityFieldKind::LegsAnim),
    field("groundEntityNum", 10, EntityFieldKind::GroundEntityNum),
    field("pos.trType", 8, EntityFieldKind::PosType),
    field("eFlags", 19, EntityFieldKind::EFlags),
    field("otherEntityNum", 10, EntityFieldKind::OtherEntityNum),
    field("weapon", 8, EntityFieldKind::Weapon),
    field("clientNum", 8, EntityFieldKind::ClientNum),
    field("angles[1]", 0, EntityFieldKind::Angles(1)),
    field("pos.trDuration", 32, EntityFieldKind::PosDuration),
    field("apos.trType", 8, EntityFieldKind::AposType),
    field("origin[0]", 0, EntityFieldKind::Origin(0)),
    field("origin[1]", 0, EntityFieldKind::Origin(1)),
    field("origin[2]", 0, EntityFieldKind::Origin(2)),
    field("solid", 24, EntityFieldKind::Solid),
    field("powerups", 16, EntityFieldKind::Powerups),
    field("modelindex", 8, EntityFieldKind::Modelindex),
    field("otherEntityNum2", 10, EntityFieldKind::OtherEntityNum2),
    field("loopSound", 8, EntityFieldKind::LoopSound),
    field("generic1", 8, EntityFieldKind::Generic1),
    field("origin2[2]", 0, EntityFieldKind::Origin2(2)),
    field("origin2[0]", 0, EntityFieldKind::Origin2(0)),
    field("origin2[1]", 0, EntityFieldKind::Origin2(1)),
    field("modelindex2", 8, EntityFieldKind::Modelindex2),
    field("angles[0]", 0, EntityFieldKind::Angles(0)),
    field("time", 32, EntityFieldKind::Time),
    field("apos.trTime", 32, EntityFieldKind::AposTime),
    field("apos.trDuration", 32, EntityFieldKind::AposDuration),
    field("apos.trBase[2]", 0, EntityFieldKind::AposBase(2)),
    field("apos.trDelta[0]", 0, EntityFieldKind::AposDelta(0)),
    field("apos.trDelta[1]", 0, EntityFieldKind::AposDelta(1)),
    field("apos.trDelta[2]", 0, EntityFieldKind::AposDelta(2)),
    field("time2", 32, EntityFieldKind::Time2),
    field("angles[2]", 0, EntityFieldKind::Angles(2)),
    field("angles2[0]", 0, EntityFieldKind::Angles2(0)),
    field("angles2[2]", 0, EntityFieldKind::Angles2(2)),
    field("constantLight", 32, EntityFieldKind::ConstantLight),
    field("frame", 16, EntityFieldKind::Frame),
];

const fn field(name: &'static str, bits: i32, kind: EntityFieldKind) -> EntityField {
    EntityField { name, bits, kind }
}

fn entity_int(state: &Q3EntityState, kind: EntityFieldKind) -> i32 {
    match kind {
        EntityFieldKind::PosTime => state.pos.time,
        EntityFieldKind::PosType => state.pos.trajectory_type,
        EntityFieldKind::PosDuration => state.pos.duration,
        EntityFieldKind::AposTime => state.apos.time,
        EntityFieldKind::AposType => state.apos.trajectory_type,
        EntityFieldKind::AposDuration => state.apos.duration,
        EntityFieldKind::Event => state.event,
        EntityFieldKind::EType => state.e_type,
        EntityFieldKind::TorsoAnim => state.torso_anim,
        EntityFieldKind::EventParm => state.event_parm,
        EntityFieldKind::LegsAnim => state.legs_anim,
        EntityFieldKind::GroundEntityNum => state.ground_entity_num,
        EntityFieldKind::EFlags => state.e_flags,
        EntityFieldKind::OtherEntityNum => state.other_entity_num,
        EntityFieldKind::Weapon => state.weapon,
        EntityFieldKind::ClientNum => state.client_num,
        EntityFieldKind::Solid => state.solid,
        EntityFieldKind::Powerups => state.powerups,
        EntityFieldKind::Modelindex => state.modelindex,
        EntityFieldKind::OtherEntityNum2 => state.other_entity_num2,
        EntityFieldKind::LoopSound => state.loop_sound,
        EntityFieldKind::Generic1 => state.generic1,
        EntityFieldKind::Modelindex2 => state.modelindex2,
        EntityFieldKind::Time => state.time,
        EntityFieldKind::Time2 => state.time2,
        EntityFieldKind::ConstantLight => state.constant_light,
        EntityFieldKind::Frame => state.frame,
        EntityFieldKind::PosBase(axis) => state.pos.base[axis].to_bits() as i32,
        EntityFieldKind::PosDelta(axis) => state.pos.delta[axis].to_bits() as i32,
        EntityFieldKind::AposBase(axis) => state.apos.base[axis].to_bits() as i32,
        EntityFieldKind::AposDelta(axis) => state.apos.delta[axis].to_bits() as i32,
        EntityFieldKind::Origin(axis) => state.origin[axis].to_bits() as i32,
        EntityFieldKind::Origin2(axis) => state.origin2[axis].to_bits() as i32,
        EntityFieldKind::Angles(axis) => state.angles[axis].to_bits() as i32,
        EntityFieldKind::Angles2(axis) => state.angles2[axis].to_bits() as i32,
    }
}

fn entity_float(state: &Q3EntityState, kind: EntityFieldKind) -> f32 {
    match kind {
        EntityFieldKind::PosBase(axis) => state.pos.base[axis],
        EntityFieldKind::PosDelta(axis) => state.pos.delta[axis],
        EntityFieldKind::AposBase(axis) => state.apos.base[axis],
        EntityFieldKind::AposDelta(axis) => state.apos.delta[axis],
        EntityFieldKind::Origin(axis) => state.origin[axis],
        EntityFieldKind::Origin2(axis) => state.origin2[axis],
        EntityFieldKind::Angles(axis) => state.angles[axis],
        EntityFieldKind::Angles2(axis) => state.angles2[axis],
        _ => f32::from_bits(entity_int(state, kind) as u32),
    }
}

fn set_entity_int(state: &mut Q3EntityState, kind: EntityFieldKind, value: i32) {
    match kind {
        EntityFieldKind::PosTime => state.pos.time = value,
        EntityFieldKind::PosType => state.pos.trajectory_type = value,
        EntityFieldKind::PosDuration => state.pos.duration = value,
        EntityFieldKind::AposTime => state.apos.time = value,
        EntityFieldKind::AposType => state.apos.trajectory_type = value,
        EntityFieldKind::AposDuration => state.apos.duration = value,
        EntityFieldKind::Event => state.event = value,
        EntityFieldKind::EType => state.e_type = value,
        EntityFieldKind::TorsoAnim => state.torso_anim = value,
        EntityFieldKind::EventParm => state.event_parm = value,
        EntityFieldKind::LegsAnim => state.legs_anim = value,
        EntityFieldKind::GroundEntityNum => state.ground_entity_num = value,
        EntityFieldKind::EFlags => state.e_flags = value,
        EntityFieldKind::OtherEntityNum => state.other_entity_num = value,
        EntityFieldKind::Weapon => state.weapon = value,
        EntityFieldKind::ClientNum => state.client_num = value,
        EntityFieldKind::Solid => state.solid = value,
        EntityFieldKind::Powerups => state.powerups = value,
        EntityFieldKind::Modelindex => state.modelindex = value,
        EntityFieldKind::OtherEntityNum2 => state.other_entity_num2 = value,
        EntityFieldKind::LoopSound => state.loop_sound = value,
        EntityFieldKind::Generic1 => state.generic1 = value,
        EntityFieldKind::Modelindex2 => state.modelindex2 = value,
        EntityFieldKind::Time => state.time = value,
        EntityFieldKind::Time2 => state.time2 = value,
        EntityFieldKind::ConstantLight => state.constant_light = value,
        EntityFieldKind::Frame => state.frame = value,
        EntityFieldKind::PosBase(axis) => state.pos.base[axis] = f32::from_bits(value as u32),
        EntityFieldKind::PosDelta(axis) => state.pos.delta[axis] = f32::from_bits(value as u32),
        EntityFieldKind::AposBase(axis) => state.apos.base[axis] = f32::from_bits(value as u32),
        EntityFieldKind::AposDelta(axis) => state.apos.delta[axis] = f32::from_bits(value as u32),
        EntityFieldKind::Origin(axis) => state.origin[axis] = f32::from_bits(value as u32),
        EntityFieldKind::Origin2(axis) => state.origin2[axis] = f32::from_bits(value as u32),
        EntityFieldKind::Angles(axis) => state.angles[axis] = f32::from_bits(value as u32),
        EntityFieldKind::Angles2(axis) => state.angles2[axis] = f32::from_bits(value as u32),
    }
}

/// One delta-coded player field.
#[derive(Debug, Clone, Copy)]
struct PlayerField {
    name: &'static str,
    bits: i32,
    kind: PlayerFieldKind,
}

#[derive(Debug, Clone, Copy)]
enum PlayerFieldKind {
    CommandTime,
    Origin(usize),
    BobCycle,
    Velocity(usize),
    Viewangles(usize),
    WeaponTime,
    LegsTimer,
    PmTime,
    EventSequence,
    TorsoAnim,
    MovementDir,
    Events(usize),
    LegsAnim,
    PmFlags,
    GroundEntityNum,
    WeaponState,
    EFlags,
    ExternalEvent,
    Gravity,
    Speed,
    DeltaAngles(usize),
    ExternalEventParm,
    Viewheight,
    DamageEvent,
    DamageYaw,
    DamagePitch,
    DamageCount,
    Generic1,
    PmType,
    TorsoTimer,
    EventParms(usize),
    ClientNum,
    Weapon,
    GrapplePoint(usize),
    JumppadEnt,
    LoopSound,
}

const fn pfield(name: &'static str, bits: i32, kind: PlayerFieldKind) -> PlayerField {
    PlayerField { name, bits, kind }
}

const PLAYER_FIELDS: &[PlayerField] = &[
    pfield("commandTime", 32, PlayerFieldKind::CommandTime),
    pfield("origin[0]", 0, PlayerFieldKind::Origin(0)),
    pfield("origin[1]", 0, PlayerFieldKind::Origin(1)),
    pfield("bobCycle", 8, PlayerFieldKind::BobCycle),
    pfield("velocity[0]", 0, PlayerFieldKind::Velocity(0)),
    pfield("velocity[1]", 0, PlayerFieldKind::Velocity(1)),
    pfield("viewangles[1]", 0, PlayerFieldKind::Viewangles(1)),
    pfield("viewangles[0]", 0, PlayerFieldKind::Viewangles(0)),
    pfield("weaponTime", -16, PlayerFieldKind::WeaponTime),
    pfield("origin[2]", 0, PlayerFieldKind::Origin(2)),
    pfield("velocity[2]", 0, PlayerFieldKind::Velocity(2)),
    pfield("legsTimer", 8, PlayerFieldKind::LegsTimer),
    pfield("pm_time", -16, PlayerFieldKind::PmTime),
    pfield("eventSequence", 16, PlayerFieldKind::EventSequence),
    pfield("torsoAnim", 8, PlayerFieldKind::TorsoAnim),
    pfield("movementDir", 4, PlayerFieldKind::MovementDir),
    pfield("events[0]", 8, PlayerFieldKind::Events(0)),
    pfield("legsAnim", 8, PlayerFieldKind::LegsAnim),
    pfield("events[1]", 8, PlayerFieldKind::Events(1)),
    pfield("pm_flags", 16, PlayerFieldKind::PmFlags),
    pfield("groundEntityNum", 10, PlayerFieldKind::GroundEntityNum),
    pfield("weaponstate", 4, PlayerFieldKind::WeaponState),
    pfield("eFlags", 16, PlayerFieldKind::EFlags),
    pfield("externalEvent", 10, PlayerFieldKind::ExternalEvent),
    pfield("gravity", 16, PlayerFieldKind::Gravity),
    pfield("speed", 16, PlayerFieldKind::Speed),
    pfield("delta_angles[1]", 16, PlayerFieldKind::DeltaAngles(1)),
    pfield("externalEventParm", 8, PlayerFieldKind::ExternalEventParm),
    pfield("viewheight", -8, PlayerFieldKind::Viewheight),
    pfield("damageEvent", 8, PlayerFieldKind::DamageEvent),
    pfield("damageYaw", 8, PlayerFieldKind::DamageYaw),
    pfield("damagePitch", 8, PlayerFieldKind::DamagePitch),
    pfield("damageCount", 8, PlayerFieldKind::DamageCount),
    pfield("generic1", 8, PlayerFieldKind::Generic1),
    pfield("pm_type", 8, PlayerFieldKind::PmType),
    pfield("delta_angles[0]", 16, PlayerFieldKind::DeltaAngles(0)),
    pfield("delta_angles[2]", 16, PlayerFieldKind::DeltaAngles(2)),
    pfield("torsoTimer", 12, PlayerFieldKind::TorsoTimer),
    pfield("eventParms[0]", 8, PlayerFieldKind::EventParms(0)),
    pfield("eventParms[1]", 8, PlayerFieldKind::EventParms(1)),
    pfield("clientNum", 8, PlayerFieldKind::ClientNum),
    pfield("weapon", 5, PlayerFieldKind::Weapon),
    pfield("viewangles[2]", 0, PlayerFieldKind::Viewangles(2)),
    pfield("grapplePoint[0]", 0, PlayerFieldKind::GrapplePoint(0)),
    pfield("grapplePoint[1]", 0, PlayerFieldKind::GrapplePoint(1)),
    pfield("grapplePoint[2]", 0, PlayerFieldKind::GrapplePoint(2)),
    pfield("jumppad_ent", 10, PlayerFieldKind::JumppadEnt),
    pfield("loopSound", 16, PlayerFieldKind::LoopSound),
];

fn player_int(state: &Q3PlayerState, kind: PlayerFieldKind) -> i32 {
    match kind {
        PlayerFieldKind::CommandTime => state.command_time,
        PlayerFieldKind::BobCycle => state.bob_cycle,
        PlayerFieldKind::WeaponTime => state.weapon_time,
        PlayerFieldKind::LegsTimer => state.legs_timer,
        PlayerFieldKind::PmTime => state.pm_time,
        PlayerFieldKind::EventSequence => state.event_sequence,
        PlayerFieldKind::TorsoAnim => state.torso_anim,
        PlayerFieldKind::MovementDir => state.movement_dir,
        PlayerFieldKind::Events(index) => state.events[index],
        PlayerFieldKind::LegsAnim => state.legs_anim,
        PlayerFieldKind::PmFlags => state.pm_flags,
        PlayerFieldKind::GroundEntityNum => state.ground_entity_num,
        PlayerFieldKind::WeaponState => state.weapon_state,
        PlayerFieldKind::EFlags => state.e_flags,
        PlayerFieldKind::ExternalEvent => state.external_event,
        PlayerFieldKind::Gravity => state.gravity,
        PlayerFieldKind::Speed => state.speed,
        PlayerFieldKind::DeltaAngles(axis) => state.delta_angles[axis] as i32,
        PlayerFieldKind::ExternalEventParm => state.external_event_parm,
        PlayerFieldKind::Viewheight => state.viewheight,
        PlayerFieldKind::DamageEvent => state.damage_event,
        PlayerFieldKind::DamageYaw => state.damage_yaw,
        PlayerFieldKind::DamagePitch => state.damage_pitch,
        PlayerFieldKind::DamageCount => state.damage_count,
        PlayerFieldKind::Generic1 => state.generic1,
        PlayerFieldKind::PmType => state.pm_type,
        PlayerFieldKind::TorsoTimer => state.torso_timer,
        PlayerFieldKind::EventParms(index) => state.event_parms[index],
        PlayerFieldKind::ClientNum => state.client_num,
        PlayerFieldKind::Weapon => state.weapon,
        PlayerFieldKind::JumppadEnt => state.jumppad_ent,
        PlayerFieldKind::LoopSound => state.loop_sound,
        PlayerFieldKind::Origin(axis) => state.origin[axis].to_bits() as i32,
        PlayerFieldKind::Velocity(axis) => state.velocity[axis].to_bits() as i32,
        PlayerFieldKind::Viewangles(axis) => state.viewangles[axis].to_bits() as i32,
        PlayerFieldKind::GrapplePoint(axis) => state.grapple_point[axis].to_bits() as i32,
    }
}

fn player_float(state: &Q3PlayerState, kind: PlayerFieldKind) -> f32 {
    match kind {
        PlayerFieldKind::Origin(axis) => state.origin[axis],
        PlayerFieldKind::Velocity(axis) => state.velocity[axis],
        PlayerFieldKind::Viewangles(axis) => state.viewangles[axis],
        PlayerFieldKind::GrapplePoint(axis) => state.grapple_point[axis],
        _ => f32::from_bits(player_int(state, kind) as u32),
    }
}

fn set_player_int(state: &mut Q3PlayerState, kind: PlayerFieldKind, value: i32) {
    match kind {
        PlayerFieldKind::CommandTime => state.command_time = value,
        PlayerFieldKind::BobCycle => state.bob_cycle = value,
        PlayerFieldKind::WeaponTime => state.weapon_time = value,
        PlayerFieldKind::LegsTimer => state.legs_timer = value,
        PlayerFieldKind::PmTime => state.pm_time = value,
        PlayerFieldKind::EventSequence => state.event_sequence = value,
        PlayerFieldKind::TorsoAnim => state.torso_anim = value,
        PlayerFieldKind::MovementDir => state.movement_dir = value,
        PlayerFieldKind::Events(index) => state.events[index] = value,
        PlayerFieldKind::LegsAnim => state.legs_anim = value,
        PlayerFieldKind::PmFlags => state.pm_flags = value,
        PlayerFieldKind::GroundEntityNum => state.ground_entity_num = value,
        PlayerFieldKind::WeaponState => state.weapon_state = value,
        PlayerFieldKind::EFlags => state.e_flags = value,
        PlayerFieldKind::ExternalEvent => state.external_event = value,
        PlayerFieldKind::Gravity => state.gravity = value,
        PlayerFieldKind::Speed => state.speed = value,
        PlayerFieldKind::DeltaAngles(axis) => state.delta_angles[axis] = value as f32,
        PlayerFieldKind::ExternalEventParm => state.external_event_parm = value,
        PlayerFieldKind::Viewheight => state.viewheight = value,
        PlayerFieldKind::DamageEvent => state.damage_event = value,
        PlayerFieldKind::DamageYaw => state.damage_yaw = value,
        PlayerFieldKind::DamagePitch => state.damage_pitch = value,
        PlayerFieldKind::DamageCount => state.damage_count = value,
        PlayerFieldKind::Generic1 => state.generic1 = value,
        PlayerFieldKind::PmType => state.pm_type = value,
        PlayerFieldKind::TorsoTimer => state.torso_timer = value,
        PlayerFieldKind::EventParms(index) => state.event_parms[index] = value,
        PlayerFieldKind::ClientNum => state.client_num = value,
        PlayerFieldKind::Weapon => state.weapon = value,
        PlayerFieldKind::JumppadEnt => state.jumppad_ent = value,
        PlayerFieldKind::LoopSound => state.loop_sound = value,
        PlayerFieldKind::Origin(axis) => state.origin[axis] = f32::from_bits(value as u32),
        PlayerFieldKind::Velocity(axis) => state.velocity[axis] = f32::from_bits(value as u32),
        PlayerFieldKind::Viewangles(axis) => state.viewangles[axis] = f32::from_bits(value as u32),
        PlayerFieldKind::GrapplePoint(axis) => state.grapple_point[axis] = f32::from_bits(value as u32),
    }
}

/// Wire metadata for delta fields (`stateDeltaFields`).
#[must_use]
pub fn state_delta_fields(kind: &str) -> Vec<(String, i32)> {
    match kind {
        "player" => PLAYER_FIELDS.iter().map(|field| (field.name.to_string(), field.bits)).collect(),
        _ => ENTITY_FIELDS.iter().map(|field| (field.name.to_string(), field.bits)).collect(),
    }
}

/// Delta-message diagnostics (`DeltaMessageDiagnostics`).
pub trait DeltaDiagnostics {
    /// `cl_shownet` level.
    fn shownet(&self) -> i32;
    /// Print a diagnostic.
    fn print(&mut self, text: &str);
    /// Read offset in bytes.
    fn offset(&self) -> usize;
}

fn diagnostic_bit_position(reader: &Q3MsgReader<'_>, offset: usize) -> isize {
    let bit = reader.bit_position() as isize + offset as isize * 8;
    let count = reader.read_count() as isize + offset as isize;
    if bit == 0 {
        count * 8 - ENTITY_NUMBER_BITS as isize
    } else {
        (count - 1) * 8 + bit - ENTITY_NUMBER_BITS as isize
    }
}

fn double_decimal(digits: &mut Vec<u8>) {
    let mut carry = 0u32;
    for digit in digits.iter_mut().rev() {
        let value = u32::from(*digit) * 2 + carry;
        *digit = (value % 10) as u8;
        carry = value / 10;
    }
    while carry > 0 {
        digits.insert(0, (carry % 10) as u8);
        carry /= 10;
    }
}

fn decimal_string(mut value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push((value % 10) as u8);
        value /= 10;
    }
    digits.reverse();
    digits
}

/// Linux `printf` `%f`: six decimal places, round-half-even (`diagnosticFloat`).
fn diagnostic_float(bits: u32) -> String {
    let exponent = (bits >> 23) & 255;
    let fraction = bits & 0x7f_ffff;
    let sign = if bits >> 31 == 0 { "" } else { "-" };
    if exponent == 255 {
        return format!("{sign}{}", if fraction == 0 { "inf" } else { "nan" });
    }
    let mantissa = if exponent == 0 { fraction } else { fraction | 0x80_0000 };
    let shift = if exponent == 0 { -149 } else { exponent as i32 - 150 };
    let coefficient = mantissa as u64 * 1_000_000;
    let digits = if shift >= 0 {
        let mut digits = decimal_string(coefficient);
        for _ in 0..shift {
            double_decimal(&mut digits);
        }
        digits
    } else {
        // The numerator fits in 44 bits, so u64 division is exact; shifts
        // past 63 always round to zero.
        let k = (-shift) as u32;
        let rounded = if k > 63 {
            0
        } else {
            let quotient = coefficient >> k;
            let remainder = coefficient & ((1u64 << k) - 1);
            let twice = remainder * 2;
            let divisor = 1u64 << k;
            quotient
                + u64::from(twice > divisor || (twice == divisor && quotient & 1 == 1))
        };
        decimal_string(rounded)
    };
    let mut text: String = digits.iter().map(|digit| (b'0' + digit) as char).collect();
    while text.len() < 7 {
        text.insert(0, '0');
    }
    let point = text.len() - 6;
    format!("{sign}{}.{}", &text[..point], &text[point..])
}

/// Server opcode names (`SERVER_OPCODE_NAMES`).
const SERVER_OPCODE_NAMES: &[&str] = &[
    "svc_bad",
    "svc_nop",
    "svc_gamestate",
    "svc_configstring",
    "svc_baseline",
    "svc_serverCommand",
    "svc_download",
    "svc_snapshot",
];

fn show_net<'d, 'x>(
    reader: &Q3MsgReader<'_>,
    label: &str,
    diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) {
    if let Some(diagnostics) = diagnostics {
        if diagnostics.shownet() >= 2 {
            let count = reader.read_count() as isize + diagnostics.offset() as isize - 1;
            diagnostics.print(&format!("{count:>3}:{label}\n"));
        }
    }
}

/// Reborrow an optional diagnostics sink for a nested call.
fn reborrow<'a, 'b, 'c>(
    diagnostics: &'a mut Option<&'b mut (dyn DeltaDiagnostics + 'c)>,
) -> Option<&'a mut (dyn DeltaDiagnostics + 'c)> {
    match diagnostics {
        Some(sink) => Some(&mut **sink),
        None => None,
    }
}

/// Diagnostics with the offset shadowed by the cursor's read offset.
struct OffsetDiagnostics<'a> {
    inner: &'a mut dyn DeltaDiagnostics,
    offset: usize,
}

impl DeltaDiagnostics for OffsetDiagnostics<'_> {
    fn shownet(&self) -> i32 {
        self.inner.shownet()
    }

    fn print(&mut self, text: &str) {
        self.inner.print(text);
    }

    fn offset(&self) -> usize {
        self.offset
    }
}

fn show_packet_entity<'d, 'x>(
    reader: &Q3MsgReader<'_>,
    label: &str,
    number: i32,
    diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) {
    if let Some(diagnostics) = diagnostics {
        if diagnostics.shownet() == 3 {
            let offset = diagnostics.offset();
            diagnostics.print(&format!(
                "{:>3}:  {label}: {number}\n",
                reader.read_count() + offset
            ));
        }
    }
}

fn write_entity_field(
    writer: &mut Q3MsgWriter,
    field: EntityField,
    from: &Q3EntityState,
    to: &Q3EntityState,
) -> Result<(), Q3NetError> {
    let changed = entity_int(from, field.kind) != entity_int(to, field.kind);
    writer.write_bits(i32::from(changed), 1)?;
    if !changed {
        return Ok(());
    }
    if field.bits == 0 {
        let value = entity_float(to, field.kind);
        writer.write_bits(i32::from(value != 0.0), 1)?;
        if value == 0.0 {
            return Ok(());
        }
        let compact = value.fract() == 0.0 && value >= -FLOAT_INT_BIAS as f32 && (value as i32) < FLOAT_INT_BIAS;
        writer.write_bits(i32::from(!compact), 1)?;
        if compact {
            writer.write_bits(value as i32 + FLOAT_INT_BIAS, FLOAT_INT_BITS)?;
        } else {
            writer.write_float(value)?;
        }
        return Ok(());
    }
    let value = entity_int(to, field.kind);
    writer.write_bits(i32::from(value != 0), 1)?;
    if value != 0 {
        writer.write_bits(value, field.bits)?;
    }
    Ok(())
}

fn read_entity_field<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    field: EntityField,
    from: &Q3EntityState,
    state: &mut Q3EntityState,
    diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) -> Result<(), Q3NetError> {
    if reader.read_bits(1)? == 0 {
        set_entity_int(state, field.kind, entity_int(from, field.kind));
        return Ok(());
    }
    let mut text = None;
    if reader.read_bits(1)? == 0 {
        set_entity_int(state, field.kind, 0);
    } else if field.bits != 0 {
        let value = reader.read_bits(field.bits)?;
        text = Some(value.to_string());
        set_entity_int(state, field.kind, value);
    } else if reader.read_bits(1)? == 0 {
        let value = reader.read_bits(FLOAT_INT_BITS)? - FLOAT_INT_BIAS;
        text = Some(value.to_string());
        set_entity_int(state, field.kind, value);
    } else {
        let bits = reader.read_bits(32)? as u32;
        if diagnostics.is_some() {
            text = Some(diagnostic_float(bits));
        }
        set_entity_int(state, field.kind, bits as i32);
    }
    if let (Some(diagnostics), Some(text)) = (diagnostics, text) {
        diagnostics.print(&format!("{}:{text} ", field.name));
    }
    Ok(())
}

fn check_entity_number(number: i32) -> Result<(), Q3NetError> {
    if number < 0 || number >= MAX_Q3_ENTITIES {
        return Err(Q3NetError::BadDeltaEntity { number });
    }
    Ok(())
}

/// Write a delta entity (`writeDeltaEntity`).
pub fn write_delta_entity(
    writer: &mut Q3MsgWriter,
    from: Option<&Q3EntityState>,
    to: Option<&Q3EntityState>,
    force: bool,
) -> Result<(), Q3NetError> {
    let Some(to) = to else {
        let Some(from) = from else {
            return Ok(());
        };
        check_entity_number(from.number)?;
        writer.write_bits(from.number, ENTITY_NUMBER_BITS)?;
        writer.write_bits(1, 1)?;
        return Ok(());
    };
    check_entity_number(to.number)?;
    let baseline;
    let from = match from {
        Some(from) => from,
        None => {
            baseline = Q3EntityState::default();
            &baseline
        }
    };
    let mut last = 0;
    for (index, field) in ENTITY_FIELDS.iter().enumerate() {
        if entity_int(from, field.kind) != entity_int(to, field.kind) {
            last = index + 1;
        }
    }
    if last == 0 && !force {
        return Ok(());
    }
    writer.write_bits(to.number, ENTITY_NUMBER_BITS)?;
    writer.write_bits(0, 1)?;
    writer.write_bits(i32::from(last != 0), 1)?;
    if last == 0 {
        return Ok(());
    }
    writer.write_byte(last as i32)?;
    for field in &ENTITY_FIELDS[..last] {
        write_entity_field(writer, *field, from, to)?;
    }
    Ok(())
}

/// Read a delta entity (`readDeltaEntity`).
pub fn read_delta_entity<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    from: &Q3EntityState,
    number: i32,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) -> Result<Q3EntityState, Q3NetError> {
    check_entity_number(number)?;
    let offset = diagnostics.as_ref().map_or(0, |diagnostics| diagnostics.offset());
    let start = diagnostic_bit_position(reader, offset);
    let mut to = Q3EntityState::default();
    if reader.read_bits(1)? != 0 {
        to.number = ENTITYNUM_NONE;
        if let Some(diagnostics) = diagnostics {
            if diagnostics.shownet() >= 2 || diagnostics.shownet() == -1 {
                let count = reader.read_count() as isize + offset as isize;
                diagnostics.print(&format!("{count:>3}: #{number:<3} remove\n"));
            }
        }
        return Ok(to);
    }
    if reader.read_bits(1)? == 0 {
        to = from.clone();
        to.number = number;
        return Ok(to);
    }
    let last = reader.read_byte()?;
    let printing = diagnostics
        .as_ref()
        .is_some_and(|diagnostics| diagnostics.shownet() >= 2 || diagnostics.shownet() == -1);
    if printing {
        if let Some(sink) = reborrow(&mut diagnostics) {
            let count = reader.read_count() as isize + offset as isize;
            sink.print(&format!("{count:>3}: #{:<3} ", to.number));
        }
    }
    let result = if printing {
        read_delta_entity_fields(reader, from, number, to, last, reborrow(&mut diagnostics))
    } else {
        read_delta_entity_fields(reader, from, number, to, last, None)
    };
    if result.is_ok() && printing {
        let end = diagnostic_bit_position(reader, offset);
        if let Some(diagnostics) = diagnostics {
            diagnostics.print(&format!(" ({} bits)\n", end - start));
        }
    }
    result
}

fn read_delta_entity_fields<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    from: &Q3EntityState,
    number: i32,
    mut to: Q3EntityState,
    last: i32,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) -> Result<Q3EntityState, Q3NetError> {
    to.number = number;
    if last as usize > ENTITY_FIELDS.len() {
        return Err(Q3NetError::EntityLastField { last });
    }
    let last = last as usize;
    for field in &ENTITY_FIELDS[..last] {
        read_entity_field(reader, *field, from, &mut to, reborrow(&mut diagnostics))?;
    }
    for field in &ENTITY_FIELDS[last..] {
        set_entity_int(&mut to, field.kind, entity_int(from, field.kind));
    }
    Ok(to)
}

fn write_player_field(
    writer: &mut Q3MsgWriter,
    field: PlayerField,
    from: &Q3PlayerState,
    to: &Q3PlayerState,
) -> Result<(), Q3NetError> {
    let changed = player_int(from, field.kind) != player_int(to, field.kind);
    writer.write_bits(i32::from(changed), 1)?;
    if !changed {
        return Ok(());
    }
    if field.bits != 0 {
        writer.write_bits(player_int(to, field.kind), field.bits)?;
        return Ok(());
    }
    let value = player_float(to, field.kind);
    let compact = value.fract() == 0.0 && value >= -FLOAT_INT_BIAS as f32 && (value as i32) < FLOAT_INT_BIAS;
    writer.write_bits(i32::from(!compact), 1)?;
    if compact {
        writer.write_bits(value as i32 + FLOAT_INT_BIAS, FLOAT_INT_BITS)?;
    } else {
        writer.write_float(value)?;
    }
    Ok(())
}

fn read_player_field<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    field: PlayerField,
    from: &Q3PlayerState,
    state: &mut Q3PlayerState,
    diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) -> Result<(), Q3NetError> {
    if reader.read_bits(1)? == 0 {
        set_player_int(state, field.kind, player_int(from, field.kind));
        return Ok(());
    }
    // Entity-only zero encoding does not apply to players (`entity: false`).
    let mut text = None;
    if field.bits != 0 {
        let value = reader.read_bits(field.bits)?;
        text = Some((value as i32).to_string());
        set_player_int(state, field.kind, value);
    } else if reader.read_bits(1)? == 0 {
        let value = reader.read_bits(FLOAT_INT_BITS)? - FLOAT_INT_BIAS;
        text = Some(value.to_string());
        set_player_int(state, field.kind, value);
    } else {
        let bits = reader.read_bits(32)? as u32;
        if diagnostics.is_some() {
            text = Some(diagnostic_float(bits));
        }
        set_player_int(state, field.kind, bits as i32);
    }
    if let (Some(diagnostics), Some(text)) = (diagnostics, text) {
        diagnostics.print(&format!("{}:{text} ", field.name));
    }
    Ok(())
}

fn slot_mask(from: &Q3PlayerSlots, to: &Q3PlayerSlots) -> Result<i32, Q3NetError> {
    let mut bits = 0;
    for index in 0..16 {
        if from.get(index)? != to.get(index)? {
            bits |= 1 << index;
        }
    }
    Ok(bits)
}

/// Write a delta player state (`writeDeltaPlayerState`).
pub fn write_delta_player_state(
    writer: &mut Q3MsgWriter,
    from: Option<&Q3PlayerState>,
    to: &Q3PlayerState,
) -> Result<(), Q3NetError> {
    if from.is_some_and(|from| from.product != to.product) {
        return Err(Q3NetError::Range("Player delta cannot cross products"));
    }
    let baseline;
    let from = match from {
        Some(from) => from,
        None => {
            baseline = Q3PlayerState::new(to.product);
            &baseline
        }
    };
    let mut last = 0;
    for (index, field) in PLAYER_FIELDS.iter().enumerate() {
        if player_int(from, field.kind) != player_int(to, field.kind) {
            last = index + 1;
        }
    }
    writer.write_byte(last as i32)?;
    for field in &PLAYER_FIELDS[..last] {
        write_player_field(writer, *field, from, to)?;
    }
    let changed = slot_mask(&from.stats, &to.stats)? != 0
        || slot_mask(&from.persistant, &to.persistant)? != 0
        || slot_mask(&from.ammo, &to.ammo)? != 0
        || slot_mask(&from.powerups, &to.powerups)? != 0;
    writer.write_bits(i32::from(changed), 1)?;
    if !changed {
        return Ok(());
    }
    for (from_slots, to_slots, width) in [
        (&from.stats, &to.stats, 16),
        (&from.persistant, &to.persistant, 16),
        (&from.ammo, &to.ammo, 16),
        (&from.powerups, &to.powerups, 32),
    ] {
        let mask = slot_mask(from_slots, to_slots)?;
        writer.write_bits(i32::from(mask != 0), 1)?;
        if mask == 0 {
            continue;
        }
        writer.write_short(mask)?;
        for index in 0..16 {
            if (mask & (1 << index)) != 0 {
                writer.write_bits(to_slots.get(index)?, width)?;
            }
        }
    }
    Ok(())
}

/// Read a delta player state (`readDeltaPlayerState`).
pub fn read_delta_player_state<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    from: Option<&Q3PlayerState>,
    product: Q3Product,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
) -> Result<Q3PlayerState, Q3NetError> {
    if from.is_some_and(|from| from.product != product) {
        return Err(Q3NetError::Range("Player delta cannot cross products"));
    }
    let baseline;
    let from = match from {
        Some(from) => from,
        None => {
            baseline = Q3PlayerState::new(product);
            &baseline
        }
    };
    let mut to = from.clone();
    let offset = diagnostics.as_ref().map_or(0, |diagnostics| diagnostics.offset());
    let start = diagnostic_bit_position(reader, offset);
    let printing = match diagnostics.as_mut() {
        Some(diagnostics) if diagnostics.shownet() >= 2 || diagnostics.shownet() == -2 => {
            let count = reader.read_count() as isize + offset as isize;
            diagnostics.print(&format!("{count:>3}: playerstate "));
            true
        }
        _ => false,
    };
    let last = reader.read_byte()?;
    if last as usize > PLAYER_FIELDS.len() {
        return Err(Q3NetError::PlayerLastField { last });
    }
    let last = last as usize;
    for field in &PLAYER_FIELDS[..last] {
        if printing {
            read_player_field(reader, *field, from, &mut to, reborrow(&mut diagnostics))?;
        } else {
            read_player_field(reader, *field, from, &mut to, None)?;
        }
    }
    for field in &PLAYER_FIELDS[last..] {
        set_player_int(&mut to, field.kind, player_int(from, field.kind));
    }
    if reader.read_bits(1)? != 0 {
        for (key, width) in [("stats", 16), ("persistant", 16), ("ammo", 16), ("powerups", 32)] {
            if reader.read_bits(1)? == 0 {
                continue;
            }
            if let Some(diagnostics) = diagnostics.as_mut() {
                if diagnostics.shownet() == 4 {
                    diagnostics.print(&format!("PS_{} ", key.to_uppercase()));
                }
            }
            let mask = reader.read_short()?;
            for index in 0..16 {
                if (mask & (1 << index)) != 0 {
                    let value = if width == 16 { reader.read_short()? } else { reader.read_long()? };
                    match key {
                        "stats" => to.stats.set(index, value)?,
                        "persistant" => to.persistant.set(index, value)?,
                        "ammo" => to.ammo.set(index, value)?,
                        _ => to.powerups.set(index, value)?,
                    }
                }
            }
        }
    }
    if printing {
        let end = diagnostic_bit_position(reader, offset);
        if let Some(diagnostics) = diagnostics {
            diagnostics.print(&format!(" ({} bits)\n", end - start));
        }
    }
    Ok(to)
}

/// Fragment payload size (`FRAGMENT_SIZE`).
pub const FRAGMENT_SIZE: usize = 1300;
/// Maximum packet length (`MAX_PACKET_LENGTH`).
pub const MAX_PACKET_LENGTH: usize = 1400;

/// Channel role (`ChannelRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRole {
    /// Client (sends qport).
    Client,
    /// Server.
    Server,
}

/// Delivery sink for transmitted packets (`ChannelDelivery`).
pub trait ChannelDelivery {
    /// Send one datagram.
    fn send(&mut self, datagram: &[u8]);
    /// Trace one transmission.
    fn trace(&mut self, message: &str);
}

/// Receive diagnostics (`ChannelDiagnostics`).
pub struct ChannelDiagnostics<'a> {
    /// Log every packet.
    pub show_packets: bool,
    /// Log drops.
    pub show_drop: bool,
    /// Remote address text.
    pub remote_address: String,
    /// Print sink.
    pub print: Box<dyn FnMut(&str) + 'a>,
}

/// Channel receive result (`ChannelResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelResult {
    /// Complete payload.
    Accepted {
        /// Sequence.
        sequence: i32,
        /// Qport (server role).
        qport: Option<u16>,
        /// Dropped packets before this one.
        dropped: i32,
        /// Payload.
        payload: Vec<u8>,
    },
    /// Fragment buffered.
    Fragment {
        /// Sequence.
        sequence: i32,
        /// Bytes received so far.
        received: usize,
    },
    /// Rejected packet.
    Rejected {
        /// Reason.
        reason: ChannelReject,
    },
}

/// Channel rejection reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelReject {
    /// Malformed header.
    Malformed,
    /// Out-of-order sequence.
    Sequence,
    /// Fragment out of order.
    FragmentOrder,
    /// Illegal fragment length.
    FragmentLength,
}

/// Quake III netchannel (`Netchannel`).
pub struct Netchannel {
    role: ChannelRole,
    qport: u16,
    transmit_qport: Box<dyn FnMut() -> u16>,
    incoming: i32,
    outgoing: i32,
    dropped: i32,
    fragment_sequence: i32,
    fragment_length: usize,
    fragments: Vec<u8>,
    unsent_buffer: Vec<u8>,
    unsent_length: usize,
    unsent_start: usize,
    unsent_fragments: bool,
    delivering: bool,
}

impl Netchannel {
    /// Build a channel.
    pub fn new(
        role: ChannelRole,
        qport: u16,
        transmit_qport: impl FnMut() -> u16 + 'static,
    ) -> Self {
        Self {
            role,
            qport,
            transmit_qport: Box::new(transmit_qport),
            incoming: 0,
            outgoing: 1,
            dropped: 0,
            fragment_sequence: 0,
            fragment_length: 0,
            fragments: vec![0; MAX_MESSAGE_LENGTH],
            unsent_buffer: vec![0; MAX_MESSAGE_LENGTH],
            unsent_length: 0,
            unsent_start: 0,
            unsent_fragments: false,
            delivering: false,
        }
    }

    /// A cleared bot channel that never passed setup (`sourceZero`).
    #[must_use]
    pub fn source_zero() -> Self {
        let mut channel = Self::new(ChannelRole::Client, 0, || 0);
        channel.outgoing = 0;
        channel
    }

    /// Incoming sequence.
    #[must_use]
    pub fn incoming_sequence(&self) -> i32 {
        self.incoming
    }

    /// Outgoing sequence.
    #[must_use]
    pub fn outgoing_sequence(&self) -> i32 {
        self.outgoing
    }

    /// Whether fragments remain unsent.
    #[must_use]
    pub fn has_unsent_fragments(&self) -> bool {
        self.unsent_fragments
    }

    /// Unsent bytes remaining.
    #[must_use]
    pub fn remaining_unsent_bytes(&self) -> usize {
        self.unsent_length - self.unsent_start
    }

    /// Role.
    #[must_use]
    pub fn role(&self) -> ChannelRole {
        self.role
    }

    /// Qport.
    #[must_use]
    pub fn qport(&self) -> u16 {
        self.qport
    }

    fn assert_delivery_entry(&self) -> Result<(), Q3NetError> {
        if self.delivering {
            return Err(Q3NetError::Protocol("Cannot reenter channel delivery"));
        }
        Ok(())
    }

    /// Begin transmitting a payload; only the first fragment goes out.
    pub fn begin_transmit(
        &mut self,
        payload: &[u8],
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        self.assert_delivery_entry()?;
        if payload.len() > MAX_MESSAGE_LENGTH {
            return Err(Q3NetError::Range("Netchannel message too large"));
        }
        if self.outgoing >= 0x7fff_ffff {
            return Err(Q3NetError::Range("Netchannel sequence exhausted; reconnect required"));
        }
        self.unsent_start = 0;
        if payload.len() >= FRAGMENT_SIZE {
            self.unsent_fragments = true;
            self.unsent_length = payload.len();
            self.unsent_buffer[..payload.len()].copy_from_slice(payload);
            self.transmit_next_fragment(delivery)?;
            return Ok(());
        }
        let packet = self.packet(payload, false, 0)?;
        self.outgoing += 1;
        self.delivering = true;
        delivery.send(&packet);
        delivery.trace(&format!(
            "{} send {:>4} : s={} ack={}\n",
            if self.role == ChannelRole::Client { "client" } else { "server" },
            packet.len(),
            self.outgoing - 1,
            self.incoming
        ));
        self.delivering = false;
        Ok(())
    }

    /// Transmit the next fragment.
    pub fn transmit_next_fragment(
        &mut self,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<bool, Q3NetError> {
        self.assert_delivery_entry()?;
        let length = FRAGMENT_SIZE.min(self.unsent_length - self.unsent_start);
        let start = self.unsent_start;
        let chunk = self.unsent_buffer[start..start + length].to_vec();
        let packet = self.packet(&chunk, true, start)?;
        self.delivering = true;
        delivery.send(&packet);
        delivery.trace(&format!(
            "{} send {:>4} : s={} fragment={},{}\n",
            if self.role == ChannelRole::Client { "client" } else { "server" },
            packet.len(),
            self.outgoing,
            start,
            length
        ));
        self.delivering = false;
        self.unsent_start += length;
        if self.unsent_start == self.unsent_length && length != FRAGMENT_SIZE {
            self.outgoing += 1;
            self.unsent_fragments = false;
        }
        Ok(true)
    }

    /// Transmit all fragments immediately (loopback/testing).
    pub fn transmit(&mut self, payload: &[u8]) -> Result<Vec<Vec<u8>>, Q3NetError> {
        struct Collect {
            packets: Vec<Vec<u8>>,
        }
        impl ChannelDelivery for Collect {
            fn send(&mut self, datagram: &[u8]) {
                self.packets.push(datagram.to_vec());
            }
            fn trace(&mut self, _message: &str) {}
        }
        let mut delivery = Collect { packets: Vec::new() };
        self.begin_transmit(payload, &mut delivery)?;
        while self.unsent_fragments {
            self.transmit_next_fragment(&mut delivery)?;
        }
        Ok(delivery.packets)
    }

    fn packet(&mut self, payload: &[u8], fragmented: bool, start: usize) -> Result<Vec<u8>, Q3NetError> {
        let mut message = Q3MsgWriter::new(MessageMode::Oob, MAX_PACKET_LENGTH)?;
        message.write_long(self.outgoing | (if fragmented { i32::MIN } else { 0 }))?;
        if self.role == ChannelRole::Client {
            let qport = (self.transmit_qport)();
            message.write_short(i32::from(qport))?;
        }
        if fragmented {
            message.write_short(start as i32)?;
            message.write_short(payload.len() as i32)?;
        }
        message.write_data(payload)?;
        Ok(message.to_bytes().to_vec())
    }

    /// Receive one packet (address/qport routing precedes this call).
    pub fn receive(
        &mut self,
        packet: &[u8],
        mut diagnostics: Option<&mut ChannelDiagnostics<'_>>,
    ) -> ChannelResult {
            let base_header = if self.role == ChannelRole::Server { 6 } else { 4 };
        if packet.len() < base_header || packet.len() > MAX_MESSAGE_LENGTH {
            return ChannelResult::Rejected { reason: ChannelReject::Malformed };
        }
        let wire_sequence = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
        let fragmented = (wire_sequence & 0x8000_0000) != 0;
        let sequence = (wire_sequence & 0x7fff_ffff) as i32;
        let qport = if self.role == ChannelRole::Server {
            Some(u16::from_le_bytes([packet[4], packet[5]]))
        } else {
            None
        };
        if fragmented && packet.len() < base_header + 4 {
            return ChannelResult::Rejected { reason: ChannelReject::Malformed };
        }
        let start = if fragmented {
            i16::from_le_bytes([packet[base_header], packet[base_header + 1]]) as i32
        } else {
            0
        };
        let length = if fragmented {
            i16::from_le_bytes([packet[base_header + 2], packet[base_header + 3]]) as i32
        } else {
            0
        };
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            if diagnostics.show_packets {
                (diagnostics.print)(&if fragmented {
                    format!(
                        "{} recv {:>4} : s={} fragment={},{}\n",
                        if self.role == ChannelRole::Client { "client" } else { "server" },
                        packet.len(),
                        sequence,
                        start,
                        length
                    )
                } else {
                    format!(
                        "{} recv {:>4} : s={}\n",
                        if self.role == ChannelRole::Client { "client" } else { "server" },
                        packet.len(),
                        sequence,
                    )
                });
            }
        }
        if sequence <= self.incoming {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                if diagnostics.show_drop || diagnostics.show_packets {
                    (diagnostics.print)(&format!(
                        "{}:Out of order packet {} at {}\n",
                        diagnostics.remote_address, sequence, self.incoming
                    ));
                }
            }
            return ChannelResult::Rejected { reason: ChannelReject::Sequence };
        }
        self.dropped = sequence - (self.incoming + 1);
        if self.dropped > 0 {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                if diagnostics.show_drop || diagnostics.show_packets {
                    (diagnostics.print)(&format!(
                        "{}:Dropped {} packets at {}\n",
                        diagnostics.remote_address, self.dropped, sequence
                    ));
                }
            }
        }
        if !fragmented {
            self.incoming = sequence;
            return ChannelResult::Accepted {
                sequence,
                qport,
                dropped: self.dropped,
                payload: packet[base_header..].to_vec(),
            };
        }
        if sequence != self.fragment_sequence {
            self.fragment_sequence = sequence;
            self.fragment_length = 0;
        }
        if start as usize != self.fragment_length {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                if diagnostics.show_drop || diagnostics.show_packets {
                    (diagnostics.print)(&format!(
                        "{}:Dropped a message fragment\n",
                        diagnostics.remote_address
                    ));
                }
            }
            return ChannelResult::Rejected { reason: ChannelReject::FragmentOrder };
        }
        let header = base_header + 4;
        if length < 0
            || length as usize > packet.len() - header
            || self.fragment_length + length as usize > MAX_MESSAGE_LENGTH
        {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                if diagnostics.show_drop || diagnostics.show_packets {
                    (diagnostics.print)(&format!(
                        "{}:illegal fragment length\n",
                        diagnostics.remote_address
                    ));
                }
            }
            return ChannelResult::Rejected { reason: ChannelReject::FragmentLength };
        }
        let length = length as usize;
        self.fragments[self.fragment_length..self.fragment_length + length]
            .copy_from_slice(&packet[header..header + length]);
        self.fragment_length += length;
        if length == FRAGMENT_SIZE {
            return ChannelResult::Fragment { sequence, received: self.fragment_length };
        }
        let payload = self.fragments[..self.fragment_length].to_vec();
        self.fragment_length = 0;
        self.incoming = sequence;
        ChannelResult::Accepted { sequence, qport, dropped: self.dropped, payload }
    }
}

/// XOR a payload with a rolling command key (`xorPayload`).
fn xor_payload(payload: &[u8], start: usize, initial_key: i32, command: &str) -> Vec<u8> {
    let mut output = payload.to_vec();
    let text = command.split('\0').next().unwrap_or("");
    let chars: Vec<char> = text.chars().collect();
    let mut key = (initial_key & 255) as u8;
    let mut index = 0;
    for (offset, byte) in output.iter_mut().enumerate().skip(start) {
        if index >= chars.len() {
            index = 0;
        }
        let char = if chars.is_empty() { 0 } else { chars[index] as u32 };
        let mapped = if char > 127 || char == 37 { 46 } else { char as u8 };
        key ^= (((u16::from(mapped)) << (offset & 1)) & 255) as u8;
        index += 1;
        *byte ^= key;
    }
    output
}

/// `CL_Encode`/`SV_Decode` on a client payload (`xorClientMessage`).
pub fn xor_client_message(
    payload: &[u8],
    challenge: i32,
    server_command: &dyn Fn(i32) -> String,
) -> Result<Vec<u8>, Q3NetError> {
    if payload.len() <= 12 {
        return Ok(payload.to_vec());
    }
    let mut reader = Q3MsgReader::new(payload, MessageMode::Bitstream)?;
    let server_id = reader.read_long()?;
    let acknowledge = reader.read_long()?;
    let reliable = reader.read_long()?;
    Ok(xor_payload(
        payload,
        12,
        challenge ^ server_id ^ acknowledge,
        &server_command(reliable),
    ))
}

/// `SV_Encode`/`CL_Decode` on a server payload (`xorServerMessage`).
#[must_use]
pub fn xor_server_message(
    payload: &[u8],
    challenge: i32,
    sequence: i32,
    client_command: &str,
) -> Vec<u8> {
    xor_payload(payload, 4, challenge ^ sequence, client_command)
}

/// Maximum reliable commands (`MAX_RELIABLE_COMMANDS`).
pub const MAX_RELIABLE_COMMANDS: usize = 64;

/// One reliable command (`ReliableCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReliableCommand {
    /// Sequence.
    pub sequence: i32,
    /// Text.
    pub text: String,
}

/// Acknowledgement outcome (`ReliableAcknowledgement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliableAcknowledgement {
    /// Acknowledged through the sequence.
    Acknowledged(i32),
    /// Clamped to the current sequence.
    Clamped(i32),
    /// Stale acknowledgement; clamped to current.
    RejectedStale(i32),
}

/// Server command append outcome (`ServerCommandAppend`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerCommandAppend {
    /// Queued.
    Queued(ReliableCommand),
    /// Overflow: sequence advanced but the slot is untouched.
    Overflow {
        /// Current sequence.
        sequence: i32,
        /// Acknowledged sequence.
        acknowledge: i32,
    },
}

fn check_sequence(value: i32) -> Result<(), Q3NetError> {
    if value < 0 {
        return Err(Q3NetError::Range("Reliable sequence must be a nonnegative int32"));
    }
    Ok(())
}

fn stored_text(text: &str) -> String {
    let end = text.find('\0').unwrap_or(text.len());
    text[..end.min(1023)].to_string()
}

/// Parse a `cs` index with C `sscanf` `%i` semantics.
fn config_string_number(text: &str) -> Result<i32, Q3NetError> {
    let rest = text.strip_prefix("cs").ok_or(Q3NetError::Range(
        "SV_ReplacePendingServerCommands: indeterminate sscanf configstring index",
    ))?;
    let rest = rest.trim_start_matches(['\t', '\n', '\u{b}', '\u{c}', '\r', ' ']);
    let (negative, digits) = match rest.as_bytes().first() {
        Some(b'+') => (false, &rest[1..]),
        Some(b'-') => (true, &rest[1..]),
        _ => (false, rest),
    };
    let (radix, digits) = if digits.starts_with("0x") || digits.starts_with("0X") {
        (16, &digits[2..])
    } else if digits.starts_with('0') && digits.len() > 1 {
        (8, digits)
    } else {
        (10, digits)
    };
    let end = digits
        .find(|c: char| c.to_digit(radix).is_none())
        .unwrap_or(digits.len());
    if end == 0 {
        return Err(Q3NetError::Range(
            "SV_ReplacePendingServerCommands: indeterminate sscanf configstring index",
        ));
    }
    let magnitude = i64::from_str_radix(&digits[..end], radix).map_err(|_| {
        Q3NetError::Range("SV_ReplacePendingServerCommands: sscanf index exceeds signed int32")
    })?;
    let value = if negative { -magnitude } else { magnitude };
    if value < i64::from(i32::MIN) || value > i64::from(i32::MAX) {
        return Err(Q3NetError::Range(
            "SV_ReplacePendingServerCommands: sscanf index exceeds signed int32",
        ));
    }
    Ok(value as i32)
}

/// Reliable command ring (`ReliableRing`).
#[derive(Debug, Clone)]
struct ReliableRing {
    current_sequence: i32,
    acknowledged_sequence: i32,
    slots: [String; MAX_RELIABLE_COMMANDS],
}

impl ReliableRing {
    fn new() -> Self {
        Self {
            current_sequence: 0,
            acknowledged_sequence: 0,
            slots: std::array::from_fn(|_| String::new()),
        }
    }

    fn sequence(&self) -> i32 {
        self.current_sequence
    }

    fn acknowledge(&self) -> i32 {
        self.acknowledged_sequence
    }

    fn outstanding(&self) -> i32 {
        self.current_sequence - self.acknowledged_sequence
    }

    fn lookup(&self, sequence: i32) -> Result<String, Q3NetError> {
        check_sequence(sequence)?;
        if sequence > self.current_sequence {
            return Err(Q3NetError::Range("Reliable lookup is ahead of generated commands"));
        }
        Ok(self.lookup_masked(sequence))
    }

    fn lookup_masked(&self, sequence: i32) -> String {
        self.slots[(sequence & (MAX_RELIABLE_COMMANDS as i32 - 1)) as usize].clone()
    }

    fn assign_acknowledgement(&mut self, sequence: i32) {
        self.acknowledged_sequence = sequence;
    }

    fn pending(&self) -> Vec<ReliableCommand> {
        let mut commands = Vec::new();
        let mut sequence = self.acknowledged_sequence + 1;
        while sequence <= self.current_sequence {
            commands.push(ReliableCommand { sequence, text: self.lookup_masked(sequence) });
            sequence += 1;
        }
        commands
    }

    fn next_sequence(&mut self) -> Result<(), Q3NetError> {
        if self.current_sequence == 0x7fff_ffff {
            return Err(Q3NetError::Range("Reliable sequence exhausted; reconnect required"));
        }
        self.current_sequence += 1;
        Ok(())
    }

    fn store(&mut self, text: &str) -> ReliableCommand {
        let command = ReliableCommand { sequence: self.current_sequence, text: stored_text(text) };
        self.slots[(self.current_sequence & (MAX_RELIABLE_COMMANDS as i32 - 1)) as usize] =
            command.text.clone();
        command
    }

    fn replace(&mut self, sequence: i32, text: &str) {
        self.slots[(sequence & (MAX_RELIABLE_COMMANDS as i32 - 1)) as usize] = stored_text(text);
    }

    fn validate_acknowledge(&self, sequence: i32) -> Result<(), Q3NetError> {
        check_sequence(sequence)?;
        if sequence > self.current_sequence {
            return Err(Q3NetError::Range("Reliable acknowledgement is ahead of generated commands"));
        }
        Ok(())
    }
}

/// Client reliable commands (`ClientReliableCommands`).
#[derive(Debug, Clone)]
pub struct ClientReliableCommands {
    ring: ReliableRing,
}

impl ClientReliableCommands {
    /// Fresh ring.
    #[must_use]
    pub fn new() -> Self {
        Self { ring: ReliableRing::new() }
    }

    /// Current sequence.
    #[must_use]
    pub fn sequence(&self) -> i32 {
        self.ring.sequence()
    }

    /// Acknowledged sequence.
    #[must_use]
    pub fn acknowledge(&self) -> i32 {
        self.ring.acknowledge()
    }

    /// Outstanding count.
    #[must_use]
    pub fn outstanding(&self) -> i32 {
        self.ring.outstanding()
    }

    /// Direct ring lookup.
    pub fn lookup(&self, sequence: i32) -> Result<String, Q3NetError> {
        self.ring.lookup(sequence)
    }

    /// Masked lookup for every signed wire value.
    #[must_use]
    pub fn lookup_masked(&self, sequence: i32) -> String {
        self.ring.lookup_masked(sequence)
    }

    /// Raw assignment.
    pub fn assign_acknowledgement(&mut self, sequence: i32) {
        self.ring.assign_acknowledgement(sequence);
    }

    /// Unacknowledged commands.
    #[must_use]
    pub fn pending(&self) -> Vec<ReliableCommand> {
        self.ring.pending()
    }

    /// Append a newline within the current slot (`CL_ChangeReliableCommand`).
    pub fn change_latest(&mut self) {
        let sequence = self.ring.sequence();
        let text = self.ring.lookup_masked(sequence);
        let clipped = text.chars().take(1022).collect::<String>();
        self.ring.replace(sequence, &format!("{clipped}\n"));
    }

    /// Add a command.
    pub fn add(&mut self, text: &str) -> Result<ReliableCommand, Q3NetError> {
        if self.ring.outstanding() > MAX_RELIABLE_COMMANDS as i32 {
            return Err(Q3NetError::ReliableOverflow {
                sequence: self.ring.sequence(),
                acknowledge: self.ring.acknowledge(),
            });
        }
        self.ring.next_sequence()?;
        Ok(self.ring.store(text))
    }

    /// Acknowledge through a sequence.
    pub fn acknowledge_through(&mut self, sequence: i32) -> Result<ReliableAcknowledgement, Q3NetError> {
        self.ring.validate_acknowledge(sequence)?;
        if sequence < self.ring.sequence() - MAX_RELIABLE_COMMANDS as i32 {
            self.ring.assign_acknowledgement(self.ring.sequence());
            return Ok(ReliableAcknowledgement::Clamped(self.ring.sequence()));
        }
        self.ring.assign_acknowledgement(sequence);
        Ok(ReliableAcknowledgement::Acknowledged(sequence))
    }
}

/// Server reliable commands (`ServerReliableCommands`).
#[derive(Debug, Clone)]
pub struct ServerReliableCommands {
    ring: ReliableRing,
}

impl ServerReliableCommands {
    /// Fresh ring.
    #[must_use]
    pub fn new() -> Self {
        Self { ring: ReliableRing::new() }
    }

    /// Current sequence.
    #[must_use]
    pub fn sequence(&self) -> i32 {
        self.ring.sequence()
    }

    /// Acknowledged sequence.
    #[must_use]
    pub fn acknowledge(&self) -> i32 {
        self.ring.acknowledge()
    }

    /// Outstanding count.
    #[must_use]
    pub fn outstanding(&self) -> i32 {
        self.ring.outstanding()
    }

    /// Direct ring lookup.
    pub fn lookup(&self, sequence: i32) -> Result<String, Q3NetError> {
        self.ring.lookup(sequence)
    }

    /// Masked lookup for every signed wire value.
    #[must_use]
    pub fn lookup_masked(&self, sequence: i32) -> String {
        self.ring.lookup_masked(sequence)
    }

    /// Raw assignment.
    pub fn assign_acknowledgement(&mut self, sequence: i32) {
        self.ring.assign_acknowledgement(sequence);
    }

    /// Unacknowledged commands.
    #[must_use]
    pub fn pending(&self) -> Vec<ReliableCommand> {
        self.ring.pending()
    }

    /// Replace a pending configstring command (`SV_ReplacePendingServerCommands`).
    pub fn replace_pending(&mut self, sent: i32, text: &str) -> Result<bool, Q3NetError> {
        let command = text.split('\0').next().unwrap_or("").to_string();
        let mut sequence = sent + 1;
        while sequence <= self.ring.sequence() {
            let pending = self.ring.lookup_masked(sequence);
            sequence += 1;
            if command.chars().take(2).collect::<String>() != pending.chars().take(2).collect::<String>() {
                continue;
            }
            if config_string_number(&command)? != config_string_number(&pending)? {
                continue;
            }
            self.ring.replace(sequence - 1, &command);
            return Ok(true);
        }
        Ok(false)
    }

    /// Add a command.
    pub fn add(&mut self, text: &str) -> Result<ServerCommandAppend, Q3NetError> {
        self.ring.next_sequence()?;
        if self.ring.outstanding() == MAX_RELIABLE_COMMANDS as i32 + 1 {
            return Ok(ServerCommandAppend::Overflow {
                sequence: self.ring.sequence(),
                acknowledge: self.ring.acknowledge(),
            });
        }
        Ok(ServerCommandAppend::Queued(self.ring.store(text)))
    }

    /// Acknowledge through a sequence.
    pub fn acknowledge_through(&mut self, sequence: i32) -> Result<ReliableAcknowledgement, Q3NetError> {
        self.ring.validate_acknowledge(sequence)?;
        if sequence < self.ring.sequence() - MAX_RELIABLE_COMMANDS as i32 {
            self.ring.assign_acknowledgement(self.ring.sequence());
            return Ok(ReliableAcknowledgement::RejectedStale(self.ring.sequence()));
        }
        self.ring.assign_acknowledgement(sequence);
        Ok(ReliableAcknowledgement::Acknowledged(sequence))
    }
}

/// Maximum connectionless packet length.
const MAX_CONNECTIONLESS_PACKET: usize = MAX_MESSAGE_LENGTH - 1;
/// Connect prefix length (`CONNECT_OFFSET`).
const CONNECT_OFFSET: usize = 12;
/// Maximum connectionless line length.
const MAX_LINE_LENGTH: usize = 1023;
/// Maximum connect userinfo length.
const MAX_CONNECT_INFO_LENGTH: usize = 1013;
/// `connect` word bytes.
const CONNECT_WORD: &[u8] = b"connect";

/// Decoded connectionless packet (`ConnectionlessPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionlessPacket {
    /// Command token.
    pub command: String,
    /// Argument tokens.
    pub arguments: Vec<String>,
    /// Sanitized line (percent becomes dot; high bytes Latin-1).
    pub line: String,
    /// Bytes after the consumed line delimiter.
    pub payload: Vec<u8>,
    /// Compression applied.
    pub compression: ConnectionlessCompression,
    /// Line ending observed.
    pub line_ending: LineEnding,
    /// Source read count in decompressed bytes.
    pub read_count: usize,
}

/// Connectionless compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionlessCompression {
    /// Plain.
    None,
    /// Adaptive Huffman.
    Adaptive,
}

/// Line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Newline.
    Newline,
    /// NUL.
    Nul,
    /// End of packet.
    End,
    /// Length limit.
    Limit,
}

/// Connectionless receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionlessReceiver {
    /// Client (never decompresses).
    Client,
    /// Server.
    Server,
}

fn latin1_bytes(text: &str, maximum: usize) -> Result<Vec<u8>, Q3NetError> {
    if text.len() > maximum {
        return Err(Q3NetError::Range("Connectionless text exceeds source bytes"));
    }
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch as u32 > 255 {
            return Err(Q3NetError::Range("Connectionless text must be Latin-1 source bytes"));
        }
        bytes.push(ch as u8);
    }
    Ok(bytes)
}

/// Encode connectionless text (`encodeConnectionlessText`).
pub fn encode_connectionless_text(text: &str) -> Result<Vec<u8>, Q3NetError> {
    let body = latin1_bytes(text.split('\0').next().unwrap_or(""), MAX_CONNECTIONLESS_PACKET - 4)?;
    let mut packet = vec![255; body.len() + 4];
    packet[4..].copy_from_slice(&body);
    Ok(packet)
}

/// Encode a connect request with a compressed userinfo tail (`encodeConnect`).
pub fn encode_connect(userinfo: &str) -> Result<Vec<u8>, Q3NetError> {
    let info = userinfo.split('\0').next().unwrap_or("");
    if info.contains('"') || info.contains('\n') {
        return Err(Q3NetError::Range("Connect userinfo cannot contain a quote or newline"));
    }
    latin1_bytes(info, MAX_CONNECT_INFO_LENGTH)?;
    let packet = encode_connectionless_text(&format!("connect \"{info}\""))?;
    let compressed = compress_adaptive(&packet[CONNECT_OFFSET..])?;
    let mut encoded = Vec::with_capacity(CONNECT_OFFSET + compressed.len());
    encoded.extend_from_slice(&packet[..CONNECT_OFFSET]);
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

/// Normalize high bytes outside quotes/comments (`commandWhitespace`).
fn command_whitespace(line: &str) -> String {
    #[derive(PartialEq)]
    enum Mode {
        Regular,
        Quoted,
        Comment,
    }
    let chars: Vec<char> = line.chars().collect();
    let mut mode = Mode::Regular;
    let mut normalized = String::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if mode == Mode::Comment {
            normalized.push(character);
            if chars.get(index) == Some(&'*')
                && chars.get(index + 1) == Some(&'/')
            {
                normalized.push('/');
                index += 1;
                mode = Mode::Regular;
            }
            // The donor checks `line.startsWith("*/", i)`: the `*` itself.
        } else if mode == Mode::Quoted {
            normalized.push(character);
            if character == '"' {
                mode = Mode::Regular;
            }
        } else {
            if chars.get(index) == Some(&'/') && chars.get(index + 1) == Some(&'/') {
                normalized.extend(chars[index..].iter());
                break;
            }
            if chars.get(index) == Some(&'/') && chars.get(index + 1) == Some(&'*') {
                normalized.push_str("/*");
                index += 1;
                mode = Mode::Comment;
            } else {
                normalized.push(if (character as u32) > 127 { ' ' } else { character });
                if character == '"' {
                    mode = Mode::Quoted;
                }
            }
        }
        index += 1;
    }
    normalized
}

/// Decode a connectionless packet (`decodeConnectionless`).
pub fn decode_connectionless(
    packet: &[u8],
    receiver: ConnectionlessReceiver,
) -> Result<ConnectionlessPacket, Q3NetError> {
    if packet.len() < 4 {
        return Err(Q3NetError::Range("Truncated connectionless marker"));
    }
    if packet.len() > MAX_CONNECTIONLESS_PACKET {
        return Err(Q3NetError::Range("Connectionless datagram exceeds source receive limit"));
    }
    if packet[..4] != [255, 255, 255, 255] {
        return Err(Q3NetError::Range("Invalid connectionless marker"));
    }
    let mut data = packet.to_vec();
    let mut compression = ConnectionlessCompression::None;
    if receiver == ConnectionlessReceiver::Server
        && packet.len() > CONNECT_OFFSET
        && packet[4..4 + CONNECT_WORD.len()] == *CONNECT_WORD
    {
        let expanded = decompress_adaptive(
            &packet[CONNECT_OFFSET..],
            MAX_MESSAGE_LENGTH - CONNECT_OFFSET,
        )
        .map_err(|_| Q3NetError::Range("Malformed compressed connectionless payload"))?;
        let mut joined = Vec::with_capacity(CONNECT_OFFSET + expanded.len());
        joined.extend_from_slice(&packet[..CONNECT_OFFSET]);
        joined.extend_from_slice(&expanded);
        data = joined;
        compression = ConnectionlessCompression::Adaptive;
    }
    // The donor reads bytes as Latin-1 chars and counts the line in chars;
    // every source byte is one char, so byte iteration matches exactly.
    let mut line = String::new();
    let mut offset = 4;
    let mut line_ending = LineEnding::Limit;
    while line.chars().count() < MAX_LINE_LENGTH {
        let Some(byte) = data.get(offset).copied() else {
            line_ending = LineEnding::End;
            break;
        };
        offset += 1;
        if byte == 0 {
            line_ending = LineEnding::Nul;
            break;
        }
        if byte == 10 {
            line_ending = LineEnding::Newline;
            break;
        }
        line.push(char::from(if byte == 37 { 46 } else { byte }));
    }
    let tokens = tokenize_command(&command_whitespace(&line), Dialect::Q3, TextMode::Source)?;
    let mut argv = tokens.argv.into_iter();
    let command = argv.next().unwrap_or_default();
    Ok(ConnectionlessPacket {
        command,
        arguments: argv.collect(),
        line,
        payload: data[offset..].to_vec(),
        compression,
        line_ending,
        read_count: offset,
    })
}

/// Demo message (`DemoMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoMessage {
    /// Sequence.
    pub sequence: i32,
    /// Payload.
    pub payload: Vec<u8>,
}

/// Demo end (`DemoEnd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoEnd {
    /// Reason.
    pub reason: DemoEndReason,
    /// Offset.
    pub offset: usize,
}

/// Demo end reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoEndReason {
    /// Terminator record.
    Terminator,
    /// End of file.
    Eof,
    /// Truncated header.
    TruncatedHeader,
    /// Truncated payload.
    TruncatedPayload,
}

/// Demo record: message or end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoRecord {
    /// Message.
    Message(DemoMessage),
    /// End.
    End(DemoEnd),
}

/// Demo message reader (`DemoMessageReader`).
pub trait DemoMessageReader {
    /// Next record, publishing a complete sequence word first.
    fn next(&mut self, on_sequence: &mut dyn FnMut(i32)) -> Result<DemoRecord, Q3NetError>;
}

/// Framed demo reader (`DemoReader`).
#[derive(Debug, Clone)]
pub struct DemoReader<'a> {
    bytes: &'a [u8],
    position: usize,
    ended: Option<DemoEnd>,
}

impl<'a> DemoReader<'a> {
    /// Build a reader.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0, ended: None }
    }

    /// Current offset.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.position
    }

    fn end(&mut self, reason: DemoEndReason, offset: usize) -> DemoRecord {
        let end = DemoEnd { reason, offset };
        self.ended = Some(end);
        DemoRecord::End(end)
    }
}

impl DemoMessageReader for DemoReader<'_> {
    fn next(&mut self, on_sequence: &mut dyn FnMut(i32)) -> Result<DemoRecord, Q3NetError> {
        if let Some(ended) = self.ended {
            return Ok(DemoRecord::End(ended));
        }
        let start = self.position;
        let remaining = self.bytes.len() - start;
        if remaining == 0 {
            return Ok(self.end(DemoEndReason::Eof, start));
        }
        if remaining < 4 {
            self.position = self.bytes.len();
            return Ok(self.end(DemoEndReason::TruncatedHeader, start));
        }
        let sequence = i32::from_le_bytes(self.bytes[start..start + 4].try_into().unwrap_or([0; 4]));
        self.position += 4;
        on_sequence(sequence);
        if remaining < 8 {
            self.position = self.bytes.len();
            return Ok(self.end(DemoEndReason::TruncatedHeader, start));
        }
        let length =
            i32::from_le_bytes(self.bytes[start + 4..start + 8].try_into().unwrap_or([0; 4]));
        self.position += 4;
        if length == -1 {
            return Ok(self.end(DemoEndReason::Terminator, start));
        }
        if length < 0 || length as usize > MAX_MESSAGE_LENGTH {
            return Err(Q3NetError::Range("Invalid demo message length"));
        }
        if length as usize > self.bytes.len() - self.position {
            self.position = self.bytes.len();
            return Ok(self.end(DemoEndReason::TruncatedPayload, start));
        }
        let payload = self.bytes[self.position..self.position + length as usize].to_vec();
        self.position += length as usize;
        Ok(DemoRecord::Message(DemoMessage { sequence, payload }))
    }
}

/// Encode demo messages with a terminator (`encodeDemo`).
pub fn encode_demo(messages: &[DemoMessage]) -> Result<Vec<u8>, Q3NetError> {
    let mut length = 8usize;
    for message in messages {
        if message.payload.len() > MAX_MESSAGE_LENGTH {
            return Err(Q3NetError::Range("Demo message exceeds MAX_MSGLEN"));
        }
        length += 8 + message.payload.len();
    }
    let mut output = vec![0u8; length];
    let mut offset = 0;
    for message in messages {
        output[offset..offset + 4].copy_from_slice(&message.sequence.to_le_bytes());
        output[offset + 4..offset + 8].copy_from_slice(&(message.payload.len() as i32).to_le_bytes());
        output[offset + 8..offset + 8 + message.payload.len()].copy_from_slice(&message.payload);
        offset += 8 + message.payload.len();
    }
    output[offset..offset + 4].copy_from_slice(&(-1i32).to_le_bytes());
    output[offset + 4..offset + 8].copy_from_slice(&(-1i32).to_le_bytes());
    Ok(output)
}

/// Encode one demo message without the terminator (`encodeDemoMessage`).
pub fn encode_demo_message(message: &DemoMessage) -> Result<Vec<u8>, Q3NetError> {
    let framed = encode_demo(std::slice::from_ref(message))?;
    Ok(framed[..framed.len() - 8].to_vec())
}

/// Demo terminator (`finishDemo`).
#[must_use]
pub fn finish_demo() -> Vec<u8> {
    vec![255; 8]
}

/// Client configstring storage (`ClientGameStateStorage`).
#[derive(Debug, Clone)]
pub struct ClientGameStateStorage {
    offsets: [i32; 1024],
    data: Vec<u8>,
    count: usize,
}

impl ClientGameStateStorage {
    /// Fresh storage.
    #[must_use]
    pub fn new() -> Self {
        Self { offsets: [0; 1024], data: vec![0; 16000], count: 0 }
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.offsets = [0; 1024];
        self.data.fill(0);
        self.count = 0;
    }

    /// Begin initial entries.
    pub fn begin_entries(&mut self) {
        self.count = 1;
    }

    /// Copy the source record.
    #[must_use]
    pub fn copy_source_record(&self) -> (Vec<i32>, Vec<u8>, usize) {
        (self.offsets.to_vec(), self.data.clone(), self.count)
    }

    /// Copy all strings.
    #[must_use]
    pub fn copy_strings(&self) -> Vec<String> {
        (0..1024).map(|index| self.get(index).unwrap_or_default().unwrap_or_default()).collect()
    }

    /// Read one entry.
    pub fn get(&self, index: usize) -> Result<Option<String>, Q3NetError> {
        let Some(offset) = self.offsets.get(index).copied() else {
            return Err(Q3NetError::Range("Invalid configstring index"));
        };
        if offset == 0 {
            return Ok(None);
        }
        let mut value = String::new();
        let mut cursor = offset as usize;
        loop {
            let Some(byte) = self.data.get(cursor).copied() else {
                return Err(Q3NetError::Range("Invalid configstring byte"));
            };
            if byte == 0 {
                return Ok(Some(value));
            }
            value.push(char::from(byte));
            cursor += 1;
        }
    }

    /// Append an initial entry (duplicates allowed).
    pub fn append(&mut self, index: usize, value: &str) -> Result<(), Q3NetError> {
        self.validate(index, value)?;
        self.append_bytes(index, value)
    }

    /// Modify one entry, rebuilding allocation order.
    pub fn modify(&mut self, index: usize, value: &str) -> Result<bool, Q3NetError> {
        self.validate(index, value)?;
        if self.get(index)?.as_deref().unwrap_or("") == value {
            return Ok(false);
        }
        let previous = self.copy_strings();
        self.clear();
        self.begin_entries();
        for (slot, old) in previous.iter().enumerate() {
            let text = if slot == index { value } else { old.as_str() };
            if !text.is_empty() {
                self.append_bytes(slot, text)?;
            }
        }
        Ok(true)
    }

    fn validate(&self, index: usize, value: &str) -> Result<(), Q3NetError> {
        if index >= self.offsets.len() {
            return Err(Q3NetError::drop("drop", "configstring > MAX_CONFIGSTRINGS"));
        }
        if value.chars().any(|ch| ch == '\0' || ch as u32 > 255) {
            return Err(Q3NetError::Range("Configstrings require non-NUL byte characters"));
        }
        Ok(())
    }

    fn append_bytes(&mut self, index: usize, value: &str) -> Result<(), Q3NetError> {
        if value.len() + 1 + self.count > self.data.len() {
            return Err(Q3NetError::drop("drop", "MAX_GAMESTATE_CHARS exceeded"));
        }
        self.offsets[index] = self.count as i32;
        for (offset, ch) in value.chars().enumerate() {
            self.data[self.count + offset] = ch as u8;
        }
        self.data[self.count + value.len()] = 0;
        self.count += value.len() + 1;
        Ok(())
    }
}

impl Default for ClientGameStateStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Archive entry for pure checksums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ArchiveEntry {
    /// Compressed byte length.
    pub byte_length: usize,
    /// CRC-32.
    pub crc32: u32,
    /// Whether this entry is a legacy PAK entry.
    pub pak_entry: bool,
}

/// Archive handle for pure checksums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ArchiveHandle {
    /// Whether the archive itself is a legacy PAK.
    pub pak_format: bool,
    /// Central-directory entries in physical order.
    pub entries: Vec<Q3ArchiveEntry>,
}

/// Pure checksums of an archive (`q3ArchiveChecksums`).
pub fn q3_archive_checksums(
    archive: &Q3ArchiveHandle,
    checksum_feed: i32,
) -> Result<(u32, u32), Q3NetError> {
    if archive.pak_format {
        return Err(Q3NetError::Range("Q3 pak checksums require a ZIP/PK3 central directory"));
    }
    let entries: Vec<&Q3ArchiveEntry> =
        archive.entries.iter().filter(|entry| entry.byte_length > 0).collect();
    let mut bytes = Vec::with_capacity(entries.len() * 4);
    for entry in &entries {
        if entry.pak_entry {
            return Err(Q3NetError::Range("Q3 ZIP directory contains a PAK entry"));
        }
        bytes.extend_from_slice(&entry.crc32.to_le_bytes());
    }
    Ok((
        md4_block_checksum(&bytes),
        md4_block_checksum_key(&bytes, checksum_feed as u32),
    ))
}

/// Pure server identity (`Q3PureServer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PureServer {
    /// Pure enabled.
    pub enabled: bool,
    /// Checksum feed.
    pub checksum_feed: i32,
    /// Feed server id.
    pub checksum_feed_server_id: i32,
    /// Cgame checksum.
    pub cgame_checksum: Option<i32>,
    /// UI checksum.
    pub ui_checksum: Option<i32>,
    /// Loaded pure checksums.
    pub loaded_pure_checksums: Vec<i32>,
}

/// Pure verification outcome (`Q3PureResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3PureResult {
    /// Ignored (disabled or outdated).
    Ignored(Q3PureIgnore),
    /// Authentic.
    Authentic,
    /// Rejected.
    Rejected(String),
}

/// Pure ignore reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3PureIgnore {
    /// Pure disabled.
    Disabled,
    /// Outdated feed.
    Outdated,
}

/// Verify a pure client command (`verifyQ3PureCommand`).
pub fn verify_q3_pure_command(
    server: &Q3PureServer,
    argv: &[String],
) -> Result<Q3PureResult, Q3NetError> {
    if !server.enabled {
        return Ok(Q3PureResult::Ignored(Q3PureIgnore::Disabled));
    }
    if native_atoi(argv.get(1).map(String::as_str).unwrap_or(""))?
        < server.checksum_feed_server_id
    {
        return Ok(Q3PureResult::Ignored(Q3PureIgnore::Outdated));
    }
    let rejected = || {
        Q3PureResult::Rejected("Unpure client detected. Invalid .PK3 files referenced!".to_string())
    };
    let (Some(cgame_checksum), Some(ui_checksum)) =
        (server.cgame_checksum, server.ui_checksum)
    else {
        return Ok(rejected());
    };
    if argv.len() < 6 || argv.len() - 5 > 1024 {
        return Ok(rejected());
    }
    let cgame = argv.get(2).cloned().unwrap_or_default();
    let ui = argv.get(3).cloned().unwrap_or_default();
    if cgame.starts_with('@')
        || native_atoi(&cgame)? != cgame_checksum
        || ui.starts_with('@')
        || native_atoi(&ui)? != ui_checksum
        || !argv.get(4).cloned().unwrap_or_default().starts_with('@')
    {
        return Ok(rejected());
    }
    let mut checksums = Vec::new();
    for arg in &argv[5..] {
        checksums.push(native_atoi(arg)?);
    }
    let reference_count = checksums.len() - 1;
    let mut unique = HashSet::new();
    for checksum in &checksums[..reference_count] {
        if !unique.insert(*checksum) {
            return Ok(rejected());
        }
    }
    let loaded: HashSet<i32> = server.loaded_pure_checksums.iter().take(1024).copied().collect();
    let mut checksum = server.checksum_feed;
    for reference in &unique {
        if !loaded.contains(reference) {
            return Ok(rejected());
        }
        checksum ^= reference;
    }
    checksum ^= reference_count as i32;
    Ok(if checksum == checksums[reference_count] {
        Q3PureResult::Authentic
    } else {
        rejected()
    })
}

/// Validate a download name (`checkQ3DownloadName`).
pub fn check_q3_download_name(name: &str) -> Result<(), Q3NetError> {
    let safe = !name.is_empty()
        && name.len() < 4096
        && !name.contains("..")
        && name.chars().all(|ch| matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '+' | '.' | '/' | '-'))
        && !name.starts_with('/')
        && !name.split('/').any(|component| component.is_empty() || component == ".")
        && name.to_lowercase().ends_with(".pk3");
    if safe {
        Ok(())
    } else {
        Err(Q3NetError::Range("Unsafe package download name"))
    }
}

/// Classify a stock package (`q3StockPackage`).
#[must_use]
pub fn q3_stock_package(name: &str) -> Option<&'static str> {
    let normalized = name.replace('\\', "/").replace(':', "/").to_lowercase();
    let (game, file) = normalized.split_once('/')?;
    if !matches!(game, "baseq3" | "missionpack") || file.contains('/') {
        return None;
    }
    let base = file.strip_suffix(".pk3").unwrap_or(file);
    if base.len() != 4 || !base.starts_with("pak") {
        return None;
    }
    if !matches!(base.as_bytes()[3], b'0'..=b'8') {
        return None;
    }
    Some(if game == "baseq3" { "baseq3" } else { "missionpack" })
}

/// Referenced server pak for package comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ServerPak {
    /// Name without extension.
    pub name: Option<String>,
    /// Checksum.
    pub checksum: u32,
}

/// Compare referenced and loaded packages (`compareQ3Packages`).
pub fn compare_q3_packages(
    referenced: &[Q3ServerPak],
    loaded_checksums: &[u32],
    exists: &dyn Fn(&str) -> bool,
    download: bool,
    capacity: usize,
) -> Result<String, Q3NetError> {
    if capacity < 1 {
        return Err(Q3NetError::Range("Package comparison needs a nonempty buffer"));
    }
    let loaded: HashSet<u32> = loaded_checksums.iter().copied().collect();
    let mut result = String::new();
    for pack in referenced {
        let Some(name) = pack.name.as_deref().filter(|name| !name.is_empty()) else {
            continue;
        };
        if loaded.contains(&pack.checksum) {
            continue;
        }
        let remote = format!("{name}.pk3");
        if q3_stock_package(&remote).is_some() {
            continue;
        }
        check_q3_download_name(&remote)?;
        let present = exists(&remote);
        let mut append = |text: &str| {
            let room = capacity.saturating_sub(1).saturating_sub(result.len());
            let end = text
                .char_indices()
                .take_while(|(index, _)| *index < room)
                .map(|(index, ch)| index + ch.len_utf8())
                .last()
                .unwrap_or(0);
            result.push_str(&text[..end]);
        };
        if download {
            append(&format!("@{remote}@"));
            if present {
                append(&format!("{name}.{:08x}.pk3", pack.checksum));
            } else {
                append(&remote);
            }
        } else {
            append(&remote);
            if present {
                append(" (local file exists with wrong checksum)");
            }
            append("\n");
        }
    }
    Ok(result)
}

/// Split a configstring value into `cs`/`bcs` commands (`q3ConfigstringCommands`).
#[must_use]
pub fn q3_configstring_commands(index: i32, value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < chars.len() {
        let end = (offset + 999).min(chars.len());
        chunks.push(chars[offset..end].iter().collect::<String>());
        offset = end;
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
        .iter()
        .enumerate()
        .map(|(part, chunk)| {
            let name = if chunks.len() == 1 {
                "cs"
            } else if part == 0 {
                "bcs0"
            } else if part == chunks.len() - 1 {
                "bcs2"
            } else {
                "bcs1"
            };
            format!("{name} {index} \"{chunk}\"")
        })
        .collect()
}

/// Parse-entity ring (`SourceParseEntities`).
#[derive(Debug, Clone)]
pub struct SourceParseEntities {
    cells: Vec<Q3EntityState>,
    /// Absolute entity number.
    pub number: i32,
}

/// Maximum parse entities (`MAX_PARSE_ENTITIES`).
pub const MAX_PARSE_ENTITIES: usize = 2048;

impl SourceParseEntities {
    /// Fresh ring.
    #[must_use]
    pub fn new() -> Self {
        Self { cells: vec![Q3EntityState::default(); MAX_PARSE_ENTITIES], number: 0 }
    }

    /// Borrow a cell by absolute index.
    pub fn at(&mut self, absolute: i32) -> &mut Q3EntityState {
        let index = (absolute as usize) & (MAX_PARSE_ENTITIES - 1);
        &mut self.cells[index]
    }

    /// Read a cell by absolute index.
    #[must_use]
    pub fn get(&self, absolute: i32) -> &Q3EntityState {
        &self.cells[(absolute as usize) & (MAX_PARSE_ENTITIES - 1)]
    }

    /// Advance the entity number.
    pub fn advance(&mut self) {
        self.number = self.number.wrapping_add(1);
    }

    /// Clear all cells.
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Q3EntityState::default();
        }
        self.number = 0;
    }
}

impl Default for SourceParseEntities {
    fn default() -> Self {
        Self::new()
    }
}

/// Server opcode (`ServerOpcode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ServerOpcode {
    /// No-op.
    Nop = 1,
    /// Gamestate.
    Gamestate = 2,
    /// Configstring.
    Configstring = 3,
    /// Baseline.
    Baseline = 4,
    /// Server command.
    Command = 5,
    /// Download.
    Download = 6,
    /// Snapshot.
    Snapshot = 7,
    /// End of message.
    Eof = 8,
}

/// Entity sentinel (`ENTITY_SENTINEL` = `ENTITYNUM_NONE`).
pub const ENTITY_SENTINEL: i32 = 1023;
/// Configstring space.
pub const MAX_CONFIGSTRINGS: usize = 1024;
/// Gamestate string storage.
pub const MAX_GAMESTATE_CHARS: usize = 16000;
/// Area mask bytes.
pub const MAX_AREA_BYTES: usize = 32;
/// Download block bytes.
pub const MAX_DOWNLOAD_BLOCK: usize = 2048;

/// Gamestate entry (`GamestateEntry`).
#[derive(Debug, Clone, PartialEq)]
pub enum GamestateEntry {
    /// Configstring.
    Configstring {
        /// Index.
        index: i32,
        /// Value.
        value: String,
    },
    /// Baseline.
    Baseline {
        /// Number.
        number: i32,
        /// Entity.
        entity: Q3EntityState,
    },
}

/// Gamestate (`Gamestate`).
#[derive(Debug, Clone, PartialEq)]
pub struct Gamestate {
    /// Command sequence.
    pub command_sequence: i32,
    /// Entries.
    pub entries: Vec<GamestateEntry>,
    /// Client number.
    pub client_number: i32,
    /// Checksum feed.
    pub checksum_feed: i32,
}

/// Snapshot (`Snapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Message number.
    pub message_number: i32,
    /// Server time.
    pub server_time: i32,
    /// Delta number.
    pub delta_number: i32,
    /// Flags.
    pub flags: i32,
    /// Server command number.
    pub server_command_number: i32,
    /// Parse-entities number.
    pub parse_entities_number: i32,
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// Player state.
    pub player_state: Q3PlayerState,
    /// Entities in append order.
    pub entities: Vec<Q3EntityState>,
}

/// Snapshot history entry (`SnapshotHistoryEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotHistoryEntry {
    /// Status.
    pub status: SnapshotStatus,
    /// Snapshot.
    pub snapshot: Snapshot,
}

/// Snapshot slot status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotStatus {
    /// Valid.
    Valid,
    /// Invalid.
    Invalid,
}

/// Snapshot validity (`SnapshotValidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotValidity {
    /// Valid.
    Valid,
    /// Invalid.
    Invalid(SnapshotInvalid),
}

/// Snapshot invalid reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotInvalid {
    /// Missing delta slot.
    MissingDelta,
    /// Delta slot invalid.
    InvalidDelta,
    /// Stale delta.
    StaleDelta,
    /// Stale entities.
    StaleEntities,
}

/// Download block (`Download`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadBlock {
    /// First block with file size.
    Start {
        /// File size.
        file_size: i32,
        /// Data.
        data: Vec<u8>,
    },
    /// Numbered chunk.
    Chunk {
        /// Number.
        number: i32,
        /// Data.
        data: Vec<u8>,
    },
    /// Error with message.
    Error {
        /// File size (-1).
        file_size: i32,
        /// Message.
        message: String,
    },
}

/// Server operation (`ServerOperation`).
#[derive(Debug, Clone, PartialEq)]
pub enum ServerOperation {
    /// No-op.
    Nop,
    /// Reliable command.
    Command {
        /// Sequence.
        sequence: i32,
        /// Text.
        text: String,
    },
    /// Gamestate.
    Gamestate(Gamestate),
    /// Download block.
    Download(DownloadBlock),
    /// Snapshot.
    Snapshot {
        /// Validity.
        validity: SnapshotValidity,
        /// Snapshot.
        snapshot: Snapshot,
    },
}

/// Server message context (`ServerMessageContext`).
pub struct ServerMessageContext<'a> {
    /// Product.
    pub product: Q3Product,
    /// Message number.
    pub message_number: i32,
    /// Reliable sequence.
    pub reliable_sequence: i32,
    /// Server command sequence.
    pub server_command_sequence: i32,
    /// Parse-entities number.
    pub parse_entities_number: i32,
    /// Baseline lookup.
    pub baseline: &'a dyn Fn(i32) -> Option<Q3EntityState>,
    /// History slot lookup.
    pub history: &'a dyn Fn(i32) -> Option<SnapshotHistoryEntry>,
}

/// Decoded server message (`ServerMessage`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerMessage {
    /// Reliable acknowledge.
    pub reliable_acknowledge: i32,
    /// Server command sequence.
    pub server_command_sequence: i32,
    /// Parse-entities number.
    pub parse_entities_number: i32,
    /// Operations.
    pub operations: Vec<ServerOperation>,
    /// Terminal.
    pub terminal: ServerTerminal,
}

/// Server message terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerTerminal {
    /// End of message.
    Eof,
    /// Terminal download error.
    DownloadError,
}

fn validate_entities(entities: &[Q3EntityState]) -> Result<(), Q3NetError> {
    if entities.len() > ENTITY_SENTINEL as usize {
        return Err(Q3NetError::Range("Too many snapshot entities"));
    }
    let mut previous = -1;
    for entity in entities {
        if entity.number <= previous || entity.number >= ENTITY_SENTINEL {
            return Err(Q3NetError::Range(
                "Snapshot entities must have sorted unique numbers below 1023",
            ));
        }
        previous = entity.number;
    }
    Ok(())
}

fn read_packet_entities<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    data_len: usize,
    previous: Option<&Snapshot>,
    baseline: &dyn Fn(i32) -> Option<Q3EntityState>,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
    mut parse_entities: Option<&mut SourceParseEntities>,
) -> Result<Vec<Q3EntityState>, Q3NetError> {
    fn old_state<'a>(
        previous: Option<&'a Snapshot>,
        parse_entities: Option<&'a SourceParseEntities>,
        index: usize,
    ) -> Option<Q3EntityState> {
        let previous = previous?;
        if index >= previous.entities.len() {
            return None;
        }
        if let Some(parse_entities) = parse_entities {
            return Some(parse_entities.get(previous.parse_entities_number + index as i32).clone());
        }
        previous.entities.get(index).cloned()
    }
    let mut output = Vec::new();
    // Borrow dance: `old_state` needs shared access while `deltaEntity`
    // mutates the ring. Resolve the previous entity first, then mutate.
    let mut old_index = 0usize;
    let mut old = old_state(previous, parse_entities.as_deref(), old_index);
    let mut old_number = old.as_ref().map_or(99999, |old| old.number);
    loop {
        let number = reader.read_bits(10)?;
        if number == ENTITY_SENTINEL {
            break;
        }
        if reader.read_count() > data_len {
            return Err(Q3NetError::Range("CL_ParsePacketEntities: end of message"));
        }
        while old.as_ref().is_some_and(|_| old_number < number) {
            show_packet_entity(reader, "unchanged", old_number, reborrow(&mut diagnostics));
            let from = old.clone().unwrap_or_default();
            if let Some(ring) = parse_entities.as_deref_mut() {
                let slot = ring.number;
                *ring.at(slot) = from.clone();
                ring.advance();
                output.push(from);
            } else {
                output.push(from);
            }
            old_index += 1;
            old = old_state(previous, parse_entities.as_deref(), old_index);
            old_number = old.as_ref().map_or(99999, |old| old.number);
        }
        if old.as_ref().is_some_and(|_| old_number == number) {
            show_packet_entity(reader, "delta", number, reborrow(&mut diagnostics));
            let from = old.clone().unwrap_or_default();
            let target = read_delta_entity(reader, &from, number, reborrow(&mut diagnostics))?;
            if target.number == ENTITY_SENTINEL {
                old_index += 1;
                old = old_state(previous, parse_entities.as_deref(), old_index);
                old_number = old.as_ref().map_or(99999, |old| old.number);
                continue;
            }
            if let Some(ring) = parse_entities.as_deref_mut() {
                let slot = ring.number;
                *ring.at(slot) = target.clone();
                ring.advance();
            }
            output.push(target);
            old_index += 1;
            old = old_state(previous, parse_entities.as_deref(), old_index);
            old_number = old.as_ref().map_or(99999, |old| old.number);
        } else {
            show_packet_entity(reader, "baseline", number, reborrow(&mut diagnostics));
            let from = baseline(number).unwrap_or_default();
            let target = read_delta_entity(reader, &from, number, reborrow(&mut diagnostics))?;
            if target.number == ENTITY_SENTINEL {
                continue;
            }
            if let Some(ring) = parse_entities.as_deref_mut() {
                let slot = ring.number;
                *ring.at(slot) = target.clone();
                ring.advance();
            }
            output.push(target);
        }
    }
    while old.is_some() {
        show_packet_entity(reader, "unchanged", old_number, reborrow(&mut diagnostics));
        let from = old.clone().unwrap_or_default();
        if let Some(ring) = parse_entities.as_deref_mut() {
            let slot = ring.number;
            *ring.at(slot) = from.clone();
            ring.advance();
            output.push(from);
        } else {
            output.push(from);
        }
        old_index += 1;
        old = old_state(previous, parse_entities.as_deref(), old_index);
        old_number = old.as_ref().map_or(99999, |old| old.number);
    }
    Ok(output)
}

fn write_packet_entities(
    writer: &mut Q3MsgWriter,
    previous: &[Q3EntityState],
    current: &[Q3EntityState],
    baseline: &dyn Fn(i32) -> Option<Q3EntityState>,
) -> Result<(), Q3NetError> {
    validate_entities(previous)?;
    validate_entities(current)?;
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < previous.len() || new_index < current.len() {
        let old = previous.get(old_index);
        let next = current.get(new_index);
        if let (Some(old), Some(next)) = (old, next) {
            if old.number == next.number {
                write_delta_entity(writer, Some(old), Some(next), false)?;
                old_index += 1;
                new_index += 1;
                continue;
            }
        }
        if let Some(next) = next {
            if old.is_none_or(|old| next.number < old.number) {
                write_delta_entity(writer, baseline(next.number).as_ref(), Some(next), true)?;
                new_index += 1;
                continue;
            }
        }
        if let Some(old) = old {
            write_delta_entity(writer, Some(old), None, true)?;
            old_index += 1;
            continue;
        }
        return Err(Q3NetError::Range("Missing packet entity during merge"));
    }
    writer.write_bits(ENTITY_SENTINEL, 10)?;
    Ok(())
}

fn gamestate_baselines(gamestate: &Gamestate) -> HashMap<i32, Q3EntityState> {
    let mut entries = HashMap::new();
    for entry in &gamestate.entries {
        if let GamestateEntry::Baseline { number, entity } = entry {
            entries.insert(*number, entity.clone());
        }
    }
    entries
}

fn read_download<T: FnMut(i32) -> i32 + ?Sized>(
    reader: &mut Q3MsgReader<'_>,
    publish_size: Option<&mut T>,
) -> Result<DownloadBlock, Q3NetError> {
    let number = reader.read_short()?;
    let mut file_size = 0;
    if number == 0 {
        file_size = reader.read_long()?;
        if let Some(publish) = publish_size {
            file_size = publish(file_size);
        }
        if file_size < 0 {
            return Ok(DownloadBlock::Error { file_size, message: reader.read_string()? });
        }
    }
    let size = reader.read_short()?;
    if size < 0 || size as usize > MAX_MESSAGE_LENGTH {
        return Err(Q3NetError::Range("Invalid download block length"));
    }
    let data = reader.read_data(size as usize)?;
    Ok(if number == 0 {
        DownloadBlock::Start { file_size, data }
    } else {
        DownloadBlock::Chunk { number, data }
    })
}

#[derive(Debug, Clone, Copy)]
struct SnapshotHeader {
    server_time: i32,
    delta_number: i32,
    flags: i32,
}

#[allow(clippy::too_many_arguments)]
fn read_snapshot<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    header: SnapshotHeader,
    context: &ServerMessageContext<'_>,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
    parse_entities: Option<&mut SourceParseEntities>,
    parse_number: i32,
    data_len: usize,
) -> Result<ServerOperation, Q3NetError> {
    let slot = if header.delta_number <= 0 {
        None
    } else {
        (context.history)(header.delta_number)
    };
    let mut validity = SnapshotValidity::Valid;
    if header.delta_number > 0 {
        if slot.is_none() {
            validity = SnapshotValidity::Invalid(SnapshotInvalid::MissingDelta);
        } else if slot.as_ref().is_some_and(|slot| slot.status == SnapshotStatus::Invalid) {
            validity = SnapshotValidity::Invalid(SnapshotInvalid::InvalidDelta);
        } else if slot.as_ref().is_some_and(|slot| slot.snapshot.message_number != header.delta_number) {
            validity = SnapshotValidity::Invalid(SnapshotInvalid::StaleDelta);
        } else if {
            let slot = slot.as_ref().expect("checked");
            let current = parse_entities.as_ref().map_or(parse_number, |ring| ring.number);
            current.wrapping_sub(slot.snapshot.parse_entities_number)
                > MAX_PARSE_ENTITIES as i32 - 128
        } {
            validity = SnapshotValidity::Invalid(SnapshotInvalid::StaleEntities);
        }
    }
    if let SnapshotValidity::Invalid(reason) = validity {
        if let Some(diagnostics) = reborrow(&mut diagnostics) {
            diagnostics.print(match reason {
                SnapshotInvalid::MissingDelta | SnapshotInvalid::InvalidDelta => {
                    "Delta from invalid frame (not supposed to happen!).\n"
                }
                SnapshotInvalid::StaleDelta => "Delta frame too old.\n",
                SnapshotInvalid::StaleEntities => "Delta parseEntitiesNum too old.\n",
            });
        }
    }
    let slot_snapshot = slot.as_ref().map(|slot| &slot.snapshot);
    finish_snapshot(reader, header, context, slot_snapshot, validity, diagnostics, parse_entities, parse_number, data_len)
}

#[allow(clippy::too_many_arguments)]
fn finish_snapshot<'d, 'x>(
    reader: &mut Q3MsgReader<'_>,
    header: SnapshotHeader,
    context: &ServerMessageContext<'_>,
    slot: Option<&Snapshot>,
    validity: SnapshotValidity,
    mut diagnostics: Option<&'d mut (dyn DeltaDiagnostics + 'x)>,
    parse_entities: Option<&mut SourceParseEntities>,
    parse_number: i32,
    data_len: usize,
) -> Result<ServerOperation, Q3NetError> {
    let length = reader.read_byte()?;
    if length < 0 || length as usize > MAX_AREA_BYTES {
        return Err(Q3NetError::Range("Snapshot area mask exceeds 32 bytes"));
    }
    let area_mask = reader.read_data(length as usize)?;
    show_net(reader, "playerstate", reborrow(&mut diagnostics));
    let player_state = read_delta_player_state(
        reader,
        slot.map(|slot| &slot.player_state),
        context.product,
        reborrow(&mut diagnostics),
    )?;
    show_net(reader, "packet entities", reborrow(&mut diagnostics));
    let parse_entities_number =
        parse_entities.as_ref().map_or(parse_number, |ring| ring.number);
    let entities = read_packet_entities(
        reader,
        data_len,
        slot,
        context.baseline,
        diagnostics,
        parse_entities,
    )?;
    Ok(ServerOperation::Snapshot {
        validity,
        snapshot: Snapshot {
            message_number: context.message_number,
            server_time: header.server_time,
            delta_number: header.delta_number,
            flags: header.flags,
            server_command_number: context.server_command_sequence,
            parse_entities_number,
            area_mask,
            player_state,
            entities,
        },
    })
}

/// Incremental server-message step (`ServerMessageStep`).
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessageStep {
    /// Reliable acknowledgement.
    Acknowledge(i32),
    /// Gamestate start.
    GamestateStart,
    /// Gamestate sequence.
    GamestateSequence(i32),
    /// Gamestate entry.
    GamestateEntry(GamestateEntry),
    /// Gamestate client number.
    GamestateClient(i32),
    /// Gamestate checksum feed.
    GamestateChecksum(i32),
    /// Gamestate end.
    GamestateEnd,
    /// Snapshot header.
    SnapshotHeader(i32),
    /// Decoded operation.
    Operation(ServerOperation),
    /// Terminal.
    End(ServerTerminal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorPhase {
    Header,
    Opcode,
    Sequence,
    Entries,
    Client,
    Checksum,
    GamestateEnd,
    End,
}

/// Incremental server-message decoder (`ServerMessageCursor`).
pub struct ServerMessageCursor<'b, 'p, 'f, 'd> {
    reader: Q3MsgReader<'b>,
    data_len: usize,
    phase: CursorPhase,
    terminal: ServerTerminal,
    data_count: usize,
    exhausted: bool,
    snapshot_header: Option<SnapshotHeader>,
    parse_entities: Option<&'p mut SourceParseEntities>,
    publish_download_size: Option<Box<dyn FnMut(i32) -> i32 + 'f>>,
    diagnostics: Option<&'d mut dyn DeltaDiagnostics>,
    read_offset: usize,
}

impl<'b, 'p, 'f, 'd> ServerMessageCursor<'b, 'p, 'f, 'd> {
    /// Build a cursor.
    pub fn new(
        bytes: &'b [u8],
        parse_entities: Option<&'p mut SourceParseEntities>,
        publish_download_size: Option<Box<dyn FnMut(i32) -> i32 + 'f>>,
    ) -> Result<Self, Q3NetError> {
        Self::with_diagnostics(bytes, parse_entities, publish_download_size, None, 0)
    }

    /// Build a cursor with diagnostics and a read offset.
    pub fn with_diagnostics(
        bytes: &'b [u8],
        parse_entities: Option<&'p mut SourceParseEntities>,
        publish_download_size: Option<Box<dyn FnMut(i32) -> i32 + 'f>>,
        diagnostics: Option<&'d mut dyn DeltaDiagnostics>,
        read_offset: usize,
    ) -> Result<Self, Q3NetError> {
        Ok(Self {
            reader: Q3MsgReader::new(bytes, MessageMode::Bitstream)?,
            data_len: bytes.len(),
            phase: CursorPhase::Header,
            terminal: ServerTerminal::Eof,
            data_count: 1,
            exhausted: false,
            snapshot_header: None,
            parse_entities,
            publish_download_size,
            diagnostics,
            read_offset,
        })
    }

    /// Clear the borrowed parse-entity ring (gamestate restart mid-message).
    pub fn clear_parse_entities(&mut self) {
        if let Some(parse_entities) = self.parse_entities.as_deref_mut() {
            parse_entities.clear();
        }
    }

    /// Current number of the borrowed parse-entity ring (0 when detached).
    #[must_use]
    pub fn parse_entities_number(&self) -> i32 {
        self.parse_entities.as_deref().map_or(0, |ring| ring.number)
    }

    fn source_long(&mut self) -> Result<i32, Q3NetError> {
        if self.exhausted {
            return Ok(-1);
        }
        match self.reader.read_long() {
            Ok(value) => {
                if self.reader.read_count() > self.data_len {
                    self.exhausted = true;
                }
                Ok(value)
            }
            Err(error) => {
                if self.reader.bit_position() < self.data_len * 8
                    && self.reader.read_count() <= self.data_len
                {
                    return Err(Q3NetError::from(error));
                }
                self.exhausted = true;
                Ok(-1)
            }
        }
    }

    /// Next step; the engine supplies its current context after each barrier.
    pub fn next(&mut self, context: &ServerMessageContext<'_>) -> Result<ServerMessageStep, Q3NetError> {
        // The cursor shadows the diagnostics offset with its read offset.
        let mut wrapped;
        let mut shadowed: Option<&mut dyn DeltaDiagnostics> =
            match self.diagnostics.as_deref_mut() {
                Some(inner) => {
                    wrapped = OffsetDiagnostics { inner, offset: self.read_offset };
                    Some(&mut wrapped)
                }
                None => None,
            };
        if let Some(header) = self.snapshot_header.take() {
            let parse_entities = self.parse_entities.as_deref_mut();
            let data_len = self.data_len;
            return Ok(ServerMessageStep::Operation(read_snapshot(
                &mut self.reader,
                header,
                context,
                shadowed,
                parse_entities,
                context.parse_entities_number,
                data_len,
            )?));
        }
        match self.phase {
            CursorPhase::Header => {
                if let Some(diagnostics) = reborrow(&mut shadowed) {
                    if diagnostics.shownet() == 1 {
                        let len = self.data_len + self.read_offset;
                        diagnostics.print(&format!("{len} "));
                    } else if diagnostics.shownet() >= 2 {
                        diagnostics.print("------------------\n");
                    }
                }
                let wire = self.source_long()?;
                self.phase = CursorPhase::Opcode;
                Ok(ServerMessageStep::Acknowledge(if wire < context.reliable_sequence - 64 {
                    context.reliable_sequence
                } else {
                    wire
                }))
            }
            CursorPhase::Sequence => {
                let sequence = self.source_long()?;
                self.data_count = 1;
                self.phase = CursorPhase::Entries;
                Ok(ServerMessageStep::GamestateSequence(sequence))
            }
            CursorPhase::Entries => {
                let opcode = if self.exhausted { -1 } else { self.reader.read_byte()? };
                if opcode == ServerOpcode::Eof as i32 {
                    self.phase = CursorPhase::Client;
                    return self.next(context);
                }
                if opcode == ServerOpcode::Configstring as i32 {
                    let index = self.reader.read_short()?;
                    if index < 0 || index as usize >= MAX_CONFIGSTRINGS {
                        return Err(Q3NetError::Range("Invalid configstring index"));
                    }
                    let value = self.reader.read_big_string()?;
                    if self.data_count + value.len() + 1 > MAX_GAMESTATE_CHARS {
                        return Err(Q3NetError::Range("Gamestate string storage exceeded"));
                    }
                    self.data_count += value.len() + 1;
                    return Ok(ServerMessageStep::GamestateEntry(GamestateEntry::Configstring {
                        index,
                        value,
                    }));
                }
                if opcode == ServerOpcode::Baseline as i32 {
                    let number = self.reader.read_bits(10)?;
                    let from = Q3EntityState::default();
                    let target = read_delta_entity(
                        &mut self.reader,
                        &from,
                        number,
                        reborrow(&mut shadowed),
                    )?;
                    return Ok(ServerMessageStep::GamestateEntry(GamestateEntry::Baseline {
                        number,
                        entity: target,
                    }));
                }
                Err(Q3NetError::GamestateOpcode { opcode })
            }
            CursorPhase::Client => {
                let number = self.source_long()?;
                self.phase = CursorPhase::Checksum;
                Ok(ServerMessageStep::GamestateClient(number))
            }
            CursorPhase::Checksum => {
                let checksum = self.source_long()?;
                self.phase = CursorPhase::GamestateEnd;
                Ok(ServerMessageStep::GamestateChecksum(checksum))
            }
            CursorPhase::GamestateEnd => {
                self.phase = CursorPhase::Opcode;
                Ok(ServerMessageStep::GamestateEnd)
            }
            CursorPhase::End => Ok(ServerMessageStep::End(self.terminal)),
            CursorPhase::Opcode => loop {
                if self.exhausted || self.reader.read_count() > self.data_len {
                    return Err(Q3NetError::Range(
                        "CL_ParseServerMessage: read past end of server message",
                    ));
                }
                let opcode = self.reader.read_byte()?;
                if opcode == ServerOpcode::Eof as i32 {
                    show_net(&self.reader, "END OF MESSAGE", reborrow(&mut shadowed));
                    self.phase = CursorPhase::End;
                    return Ok(ServerMessageStep::End(ServerTerminal::Eof));
                }
                if shadowed.as_ref().is_some_and(|diagnostics| diagnostics.shownet() >= 2) {
                    let name = (opcode >= 0)
                        .then(|| SERVER_OPCODE_NAMES.get(opcode as usize).copied())
                        .flatten();
                    match name {
                        None => {
                            if let Some(diagnostics) = reborrow(&mut shadowed) {
                                let count = self.reader.read_count() as isize
                                    + self.read_offset as isize
                                    - 1;
                                diagnostics.print(&format!(
                                    "{count:>3}:BAD CMD {opcode}\n"
                                ));
                            }
                        }
                        Some(name) => {
                            show_net(&self.reader, name, reborrow(&mut shadowed));
                        }
                    }
                }
                if opcode == ServerOpcode::Nop as i32 {
                    return Ok(ServerMessageStep::Operation(ServerOperation::Nop));
                }
                if opcode == ServerOpcode::Command as i32 {
                    let sequence = self.reader.read_long()?;
                    let text = self.reader.read_string()?;
                    if sequence <= context.server_command_sequence {
                        continue;
                    }
                    return Ok(ServerMessageStep::Operation(ServerOperation::Command {
                        sequence,
                        text,
                    }));
                }
                if opcode == ServerOpcode::Gamestate as i32 {
                    self.phase = CursorPhase::Sequence;
                    return Ok(ServerMessageStep::GamestateStart);
                }
                if opcode == ServerOpcode::Download as i32 {
                    let publish = self.publish_download_size.as_deref_mut();
                    let block = read_download(&mut self.reader, publish)?;
                    if matches!(block, DownloadBlock::Error { .. }) {
                        self.phase = CursorPhase::End;
                        self.terminal = ServerTerminal::DownloadError;
                    }
                    return Ok(ServerMessageStep::Operation(ServerOperation::Download(block)));
                }
                if opcode == ServerOpcode::Snapshot as i32 {
                    let server_time = self.reader.read_long()?;
                    let distance = self.reader.read_byte()?;
                    let delta_number =
                        if distance == 0 { -1 } else { context.message_number - distance };
                    let flags = self.reader.read_byte()?;
                    self.snapshot_header = Some(SnapshotHeader { server_time, delta_number, flags });
                    return Ok(ServerMessageStep::SnapshotHeader(delta_number));
                }
                return Err(Q3NetError::ServerOpcode { opcode });
            },
        }
    }
}

/// Decode a whole server message (`decodeServerMessage`).
pub fn decode_server_message(
    bytes: &[u8],
    context: &ServerMessageContext<'_>,
) -> Result<ServerMessage, Q3NetError> {
    let mut cursor = ServerMessageCursor::new(bytes, None, None)?;
    let mut reliable_acknowledge = 0;
    let mut command_sequence = context.server_command_sequence;
    let mut parse_entities_number = context.parse_entities_number;
    let null_history = |_number: i32| None;
    let mut baselines: Option<HashMap<i32, Q3EntityState>> = None;
    let mut use_history = true;
    let mut entries = Vec::new();
    let mut gamestate_sequence = 0;
    let mut client_number = 0;
    let mut checksum_feed = 0;
    let mut operations = Vec::new();
    loop {
        let map_lookup;
        let baseline: &dyn Fn(i32) -> Option<Q3EntityState> = match baselines.as_ref() {
            Some(map) => {
                map_lookup = |number: i32| map.get(&number).cloned();
                &map_lookup
            }
            None => context.baseline,
        };
        let history: &dyn Fn(i32) -> Option<SnapshotHistoryEntry> =
            if use_history { context.history } else { &null_history };
        let step = cursor.next(&ServerMessageContext {
            product: context.product,
            message_number: context.message_number,
            reliable_sequence: context.reliable_sequence,
            server_command_sequence: command_sequence,
            parse_entities_number,
            baseline,
            history,
        })?;
        match step {
            ServerMessageStep::Acknowledge(sequence) => reliable_acknowledge = sequence,
            ServerMessageStep::GamestateStart => {
                entries = Vec::new();
                parse_entities_number = 0;
                baselines = Some(gamestate_baselines(&Gamestate {
                    command_sequence: gamestate_sequence,
                    entries: entries.clone(),
                    client_number,
                    checksum_feed,
                }));
                use_history = false;
            }
            ServerMessageStep::GamestateSequence(sequence) => {
                gamestate_sequence = sequence;
                command_sequence = sequence;
            }
            ServerMessageStep::GamestateEntry(entry) => entries.push(entry),
            ServerMessageStep::GamestateClient(number) => client_number = number,
            ServerMessageStep::GamestateChecksum(checksum) => checksum_feed = checksum,
            ServerMessageStep::SnapshotHeader(_) => {}
            ServerMessageStep::GamestateEnd => {
                let gamestate = Gamestate {
                    command_sequence: gamestate_sequence,
                    entries: entries.clone(),
                    client_number,
                    checksum_feed,
                };
                operations.push(ServerOperation::Gamestate(gamestate.clone()));
                baselines = Some(gamestate_baselines(&gamestate));
                use_history = false;
            }
            ServerMessageStep::Operation(operation) => {
                if let ServerOperation::Command { sequence, .. } = &operation {
                    command_sequence = *sequence;
                }
                if let ServerOperation::Snapshot { snapshot, .. } = &operation {
                    parse_entities_number =
                        parse_entities_number.wrapping_add(snapshot.entities.len() as i32);
                }
                operations.push(operation);
            }
            ServerMessageStep::End(terminal) => {
                return Ok(ServerMessage {
                    reliable_acknowledge,
                    server_command_sequence: command_sequence,
                    parse_entities_number,
                    operations,
                    terminal,
                });
            }
        }
    }
}

fn write_download_block(writer: &mut Q3MsgWriter, block: &DownloadBlock) -> Result<(), Q3NetError> {
    writer.write_byte(ServerOpcode::Download as i32)?;
    match block {
        DownloadBlock::Error { file_size, message } => {
            if *file_size >= 0 {
                return Err(Q3NetError::Range("Invalid download error file size"));
            }
            writer.write_short(0)?;
            writer.write_long(*file_size)?;
            writer.write_string(Some(message))?;
            return Ok(());
        }
        DownloadBlock::Start { file_size, data } => {
            if *file_size < 0 {
                return Err(Q3NetError::Range("Invalid download file size"));
            }
            if data.len() > MAX_DOWNLOAD_BLOCK {
                return Err(Q3NetError::Range("Download block exceeds 2048 bytes"));
            }
            writer.write_short(0)?;
            writer.write_long(*file_size)?;
            writer.write_short(data.len() as i32)?;
            writer.write_data(data)?;
        }
        DownloadBlock::Chunk { number, data } => {
            if *number == 0 || *number < -32768 || *number > 32767 {
                return Err(Q3NetError::Range("Invalid download block number"));
            }
            if data.len() > MAX_DOWNLOAD_BLOCK {
                return Err(Q3NetError::Range("Download block exceeds 2048 bytes"));
            }
            writer.write_short(*number)?;
            writer.write_short(data.len() as i32)?;
            writer.write_data(data)?;
        }
    }
    Ok(())
}

fn write_gamestate(writer: &mut Q3MsgWriter, gamestate: &Gamestate) -> Result<(), Q3NetError> {
    writer.write_byte(ServerOpcode::Gamestate as i32)?;
    writer.write_long(gamestate.command_sequence)?;
    let mut data_count = 1;
    for entry in &gamestate.entries {
        match entry {
            GamestateEntry::Configstring { index, value } => {
                if *index < 0 || *index as usize >= MAX_CONFIGSTRINGS {
                    return Err(Q3NetError::Range("Invalid configstring index"));
                }
                let length = value.split('\0').next().map_or(0, str::len);
                if length >= 8192 {
                    return Err(Q3NetError::Range("Configstring exceeds BIG_INFO_STRING"));
                }
                data_count += length + 1;
                if data_count > MAX_GAMESTATE_CHARS {
                    return Err(Q3NetError::Range("Gamestate string storage exceeded"));
                }
                writer.write_byte(ServerOpcode::Configstring as i32)?;
                writer.write_short(*index)?;
                writer.write_big_string(Some(value))?;
            }
            GamestateEntry::Baseline { number, entity } => {
                if *number < 0 || *number > ENTITY_SENTINEL {
                    return Err(Q3NetError::Range("Invalid baseline number"));
                }
                writer.write_byte(ServerOpcode::Baseline as i32)?;
                // The entity number rides inside `writeDeltaEntity`.
                if entity.number == ENTITY_SENTINEL && *number != ENTITY_SENTINEL {
                    let mut removed = Q3EntityState::default();
                    removed.number = *number;
                    write_delta_entity(writer, Some(&removed), None, false)?;
                } else {
                    if entity.number != *number {
                        return Err(Q3NetError::Range("Baseline index differs from entity number"));
                    }
                    write_delta_entity(writer, None, Some(entity), true)?;
                }
            }
        }
    }
    writer.write_byte(ServerOpcode::Eof as i32)?;
    writer.write_long(gamestate.client_number)?;
    writer.write_long(gamestate.checksum_feed)?;
    Ok(())
}

fn write_server_snapshot(
    writer: &mut Q3MsgWriter,
    snapshot: &Snapshot,
    context: &ServerMessageContext<'_>,
    baseline: &dyn Fn(i32) -> Option<Q3EntityState>,
    history: &dyn Fn(i32) -> Option<SnapshotHistoryEntry>,
) -> Result<(), Q3NetError> {
    if snapshot.message_number != context.message_number {
        return Err(Q3NetError::Range("Snapshot message number differs from envelope"));
    }
    if snapshot.player_state.product != context.product {
        return Err(Q3NetError::Range("Snapshot product differs from envelope"));
    }
    let distance = if snapshot.delta_number <= 0 {
        0
    } else {
        snapshot.message_number.wrapping_sub(snapshot.delta_number)
    };
    if distance < 0 || distance > 255 || (distance == 0 && snapshot.delta_number > 0) {
        return Err(Q3NetError::Range("Invalid snapshot delta distance"));
    }
    let old = if distance == 0 { None } else { history(snapshot.delta_number) };
    if distance != 0
        && old.as_ref().is_none_or(|old| {
            old.status != SnapshotStatus::Valid
                || old.snapshot.message_number != snapshot.delta_number
        })
    {
        return Err(Q3NetError::Range("Missing valid snapshot baseline for encoding"));
    }
    if snapshot.area_mask.len() > MAX_AREA_BYTES {
        return Err(Q3NetError::Range("Snapshot area mask exceeds 32 bytes"));
    }
    if snapshot.flags < 0 || snapshot.flags > 255 {
        return Err(Q3NetError::Range("Invalid snapshot flags"));
    }
    writer.write_byte(ServerOpcode::Snapshot as i32)?;
    writer.write_long(snapshot.server_time)?;
    writer.write_byte(distance)?;
    writer.write_byte(snapshot.flags)?;
    let previous = old.as_ref().map(|old| &old.snapshot);
    writer.write_byte(snapshot.area_mask.len() as i32)?;
    writer.write_data(&snapshot.area_mask)?;
    write_delta_player_state(
        writer,
        previous.map(|previous| &previous.player_state),
        &snapshot.player_state,
    )?;
    write_packet_entities(
        writer,
        previous.map_or(&[], |previous| &previous.entities),
        &snapshot.entities,
        baseline,
    )?;
    Ok(())
}

/// Write server operations (`writeServerMessage`).
pub fn write_server_message(
    writer: &mut Q3MsgWriter,
    reliable_acknowledge: i32,
    operations: &[ServerOperation],
    context: &ServerMessageContext<'_>,
) -> Result<(), Q3NetError> {
    writer.write_long(reliable_acknowledge)?;
    let mut baselines: Option<HashMap<i32, Q3EntityState>> = None;
    let mut use_history = true;
    let mut download_error = false;
    for operation in operations {
        if download_error {
            return Err(Q3NetError::Range("Operations cannot follow a terminal download error"));
        }
        let map_lookup;
        let baseline: &dyn Fn(i32) -> Option<Q3EntityState> = match baselines.as_ref() {
            Some(map) => {
                map_lookup = |number: i32| map.get(&number).cloned();
                &map_lookup
            }
            None => context.baseline,
        };
        let null_history = |_number: i32| None;
        let history: &dyn Fn(i32) -> Option<SnapshotHistoryEntry> =
            if use_history { context.history } else { &null_history };
        match operation {
            ServerOperation::Nop => writer.write_byte(ServerOpcode::Nop as i32)?,
            ServerOperation::Command { sequence, text } => {
                if text.len() >= 1024 {
                    return Err(Q3NetError::Range("Server command exceeds MAX_STRING_CHARS"));
                }
                writer.write_byte(ServerOpcode::Command as i32)?;
                writer.write_long(*sequence)?;
                writer.write_string(Some(text))?;
            }
            ServerOperation::Gamestate(gamestate) => {
                write_gamestate(writer, gamestate)?;
                baselines = Some(gamestate_baselines(gamestate));
                use_history = false;
            }
            ServerOperation::Download(block) => {
                write_download_block(writer, block)?;
                download_error = matches!(block, DownloadBlock::Error { .. });
            }
            ServerOperation::Snapshot { validity, snapshot } => {
                if *validity != SnapshotValidity::Valid {
                    return Err(Q3NetError::Range("Cannot encode an invalid decoded snapshot"));
                }
                write_server_snapshot(writer, snapshot, context, baseline, history)?;
            }
        }
    }
    Ok(())
}

/// Encode a whole server message (`encodeServerMessage`).
pub fn encode_server_message(
    reliable_acknowledge: i32,
    operations: &[ServerOperation],
    context: &ServerMessageContext<'_>,
) -> Result<Vec<u8>, Q3NetError> {
    let mut writer = Q3MsgWriter::new(MessageMode::Bitstream, MAX_MESSAGE_LENGTH)?;
    write_server_message(&mut writer, reliable_acknowledge, operations, context)?;
    writer.write_byte(ServerOpcode::Eof as i32)?;
    if writer.overflowed() {
        return Err(Q3NetError::Range("Server message exceeds MAX_MSGLEN"));
    }
    Ok(writer.to_bytes().to_vec())
}

/// Snapshot backup slots (`SNAPSHOT_BACKUP`).
pub const SNAPSHOT_BACKUP: usize = 32;

/// Copy a snapshot with detached storage (`copySnapshot`).
#[must_use]
pub fn copy_snapshot(snapshot: &Snapshot) -> Snapshot {
    snapshot.clone()
}

/// Retained snapshot slot.
#[derive(Debug, Clone, PartialEq)]
struct RetainedSnapshotEntry {
    status: SnapshotStatus,
    snapshot: Snapshot,
    present: bool,
}

fn zero_snapshot(product: Q3Product) -> Snapshot {
    Snapshot {
        message_number: 0,
        server_time: 0,
        delta_number: 0,
        flags: 0,
        server_command_number: 0,
        parse_entities_number: 0,
        area_mask: vec![0; MAX_AREA_BYTES],
        player_state: Q3PlayerState::new(product),
        entities: Vec::new(),
    }
}

fn copy_into_snapshot(destination: &mut Snapshot, source: &Snapshot) {
    destination.message_number = source.message_number;
    destination.server_time = source.server_time;
    destination.delta_number = source.delta_number;
    destination.flags = source.flags;
    destination.server_command_number = source.server_command_number;
    destination.parse_entities_number = source.parse_entities_number;
    destination.area_mask = source.area_mask.clone();
    destination.player_state = source.player_state.clone();
    destination.entities = source.entities.clone();
}

/// Snapshot history ring (`SnapshotHistory`).
#[derive(Debug)]
pub struct SnapshotHistory<'a> {
    slots: Vec<Option<RetainedSnapshotEntry>>,
    current: Option<Snapshot>,
    parse_entities: Option<&'a SourceParseEntities>,
}

impl<'a> SnapshotHistory<'a> {
    /// Build history, optionally over shared parse entities.
    #[must_use]
    pub fn new(parse_entities: Option<&'a SourceParseEntities>) -> Self {
        Self { slots: vec![None; SNAPSHOT_BACKUP], current: None, parse_entities }
    }

    /// Latest published snapshot (detached copy).
    #[must_use]
    pub fn latest(&self) -> Option<Snapshot> {
        self.current.clone()
    }

    /// Current player state (detached copy).
    #[must_use]
    pub fn read_current_player_state(&self) -> Option<Q3PlayerState> {
        self.current.as_ref().map(|current| current.player_state.clone())
    }

    /// Detached history inspection.
    pub fn read_slot(&self, message_number: i32) -> Result<Option<SnapshotHistoryEntry>, Q3NetError> {
        let entry = self
            .slots
            .get((message_number & (SNAPSHOT_BACKUP as i32 - 1)) as usize)
            .ok_or(Q3NetError::Range("Missing snapshot ring slot"))?;
        Ok(match entry {
            Some(entry) if entry.present => Some(SnapshotHistoryEntry {
                status: entry.status,
                snapshot: entry.snapshot.clone(),
            }),
            _ => None,
        })
    }

    /// Borrow a slot, retaining a zero invalid record before first publication.
    pub fn borrow_slot(
        &mut self,
        message_number: i32,
        product: Q3Product,
    ) -> Result<SnapshotHistoryEntry, Q3NetError> {
        let index = (message_number & (SNAPSHOT_BACKUP as i32 - 1)) as usize;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(Q3NetError::Range("Missing snapshot ring slot"));
        };
        if slot.is_none() {
            *slot = Some(RetainedSnapshotEntry {
                status: SnapshotStatus::Invalid,
                snapshot: zero_snapshot(product),
                present: false,
            });
        } else if let Some(entry) = slot.as_mut() {
            if !entry.present && entry.snapshot.player_state.product != product {
                entry.snapshot.player_state = Q3PlayerState::new(product);
            }
        }
        let entry = slot.as_ref().expect("set");
        Ok(SnapshotHistoryEntry { status: entry.status, snapshot: entry.snapshot.clone() })
    }

    /// Non-mutating slot lookup for shared decode contexts.
    ///
    /// Returns exactly what [`borrow_slot`](Self::borrow_slot) returns: the
    /// retained zero record is never observable (`read_slot` reports it as
    /// absent and later lookups rebuild the same zero value), so `Fn`
    /// closures use this instead of borrowing the history mutably.
    pub fn read_or_zero(
        &self,
        message_number: i32,
        product: Q3Product,
    ) -> Result<SnapshotHistoryEntry, Q3NetError> {
        let index = (message_number & (SNAPSHOT_BACKUP as i32 - 1)) as usize;
        let slot = self
            .slots
            .get(index)
            .ok_or(Q3NetError::Range("Missing snapshot ring slot"))?;
        match slot {
            Some(entry)
                if entry.present || entry.snapshot.player_state.product == product =>
            {
                Ok(SnapshotHistoryEntry {
                    status: entry.status,
                    snapshot: entry.snapshot.clone(),
                })
            }
            Some(entry) => {
                let mut snapshot = entry.snapshot.clone();
                snapshot.player_state = Q3PlayerState::new(product);
                Ok(SnapshotHistoryEntry { status: entry.status, snapshot })
            }
            None => Ok(SnapshotHistoryEntry {
                status: SnapshotStatus::Invalid,
                snapshot: zero_snapshot(product),
            }),
        }
    }

    /// Publish a decoded snapshot operation.
    pub fn publish(&mut self, operation: &ServerOperation) -> Result<bool, Q3NetError> {
        let ServerOperation::Snapshot { validity, snapshot } = operation else {
            return Err(Q3NetError::Range("Snapshot history publishes snapshots"));
        };
        if *validity != SnapshotValidity::Valid {
            return Ok(false);
        }
        let previous_number = self.current.as_ref().map_or(0, |current| current.message_number);
        let incoming = snapshot.clone();
        let mut skipped = previous_number.wrapping_add(1);
        if incoming.message_number.wrapping_sub(skipped) >= SNAPSHOT_BACKUP as i32 {
            skipped = incoming.message_number - (SNAPSHOT_BACKUP as i32 - 1);
        }
        let mut cleared = 0;
        while skipped < incoming.message_number && cleared < SNAPSHOT_BACKUP {
            let index = (skipped & (SNAPSHOT_BACKUP as i32 - 1)) as usize;
            let Some(slot) = self.slots.get_mut(index) else {
                return Err(Q3NetError::Range("Missing snapshot ring slot"));
            };
            if let Some(entry) = slot.as_mut() {
                entry.status = SnapshotStatus::Invalid;
            }
            skipped = skipped.wrapping_add(1);
            cleared += 1;
        }
        self.current = Some(incoming.clone());
        let index = (incoming.message_number & (SNAPSHOT_BACKUP as i32 - 1)) as usize;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(Q3NetError::Range("Missing snapshot ring slot"));
        };
        if slot.is_none() {
            let mut retained = zero_snapshot(incoming.player_state.product);
            copy_into_snapshot(&mut retained, &incoming);
            *slot = Some(RetainedSnapshotEntry {
                status: SnapshotStatus::Valid,
                snapshot: retained,
                present: true,
            });
        } else if let Some(entry) = slot.as_mut() {
            copy_into_snapshot(&mut entry.snapshot, &incoming);
            entry.status = SnapshotStatus::Valid;
            entry.present = true;
        }
        Ok(true)
    }

    /// Clear all slots.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            if let Some(entry) = slot.as_mut() {
                let zero = zero_snapshot(entry.snapshot.player_state.product);
                copy_into_snapshot(&mut entry.snapshot, &zero);
                entry.status = SnapshotStatus::Invalid;
                entry.present = false;
            }
        }
        self.current = None;
    }

    /// Shared parse entities.
    #[must_use]
    pub fn parse_entities(&self) -> Option<&SourceParseEntities> {
        self.parse_entities
    }
}

/// Server download file (`Q3DownloadReadFile`).
pub trait Q3DownloadReadFile {
    /// File size.
    fn size(&self) -> i64;
    /// Read into target, returning the byte count.
    fn read(&mut self, target: &mut [u8]) -> usize;
    /// Close the file.
    fn close(&mut self);
}

/// Server download bindings (`Q3DownloadServerBindings`).
pub trait Q3DownloadServerBindings {
    /// Open a file.
    fn open(&mut self, name: &str) -> Option<Box<dyn Q3DownloadReadFile>>;
    /// Whether downloads are enabled.
    fn enabled(&self) -> bool;
    /// Whether the server is pure.
    fn pure(&self) -> bool;
    /// Drop the client.
    fn drop_client(&mut self, reason: &str);
    /// Print.
    fn print(&mut self, text: &str);
}

/// Download rate settings (`Q3DownloadRate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DownloadRate {
    /// Rate.
    pub rate: i32,
    /// Maximum rate.
    pub max_rate: i32,
    /// Snapshot milliseconds.
    pub snapshot_msec: i32,
}

/// Server download state (`Q3ServerDownload`).
pub struct Q3ServerDownload<'a> {
    bindings: &'a mut dyn Q3DownloadServerBindings,
    /// Current file name.
    pub name: String,
    file: Option<Box<dyn Q3DownloadReadFile>>,
    blocks: Vec<Vec<u8>>,
    block_sizes: [i32; 8],
    size: i64,
    count: i64,
    current_block: i32,
    client_block: i32,
    xmit_block: i32,
    eof: bool,
    send_time: i32,
}

impl<'a> Q3ServerDownload<'a> {
    /// Build a download over bindings.
    pub fn new(bindings: &'a mut dyn Q3DownloadServerBindings) -> Self {
        Self {
            bindings,
            name: String::new(),
            file: None,
            blocks: vec![vec![0; MAX_DOWNLOAD_BLOCK]; 8],
            block_sizes: [0; 8],
            size: 0,
            count: 0,
            current_block: 0,
            client_block: 0,
            xmit_block: 0,
            eof: false,
            send_time: 0,
        }
    }

    /// Begin a download, truncating the name at NUL/63 chars.
    pub fn begin(&mut self, name: &str) {
        self.close();
        let nul = name.find('\0').unwrap_or(name.len());
        self.name = name[..nul].chars().take(63).collect();
    }

    /// Close the current download.
    pub fn close(&mut self) {
        if let Some(mut file) = self.file.take() {
            file.close();
        }
        self.name.clear();
    }

    /// Acknowledge one block.
    pub fn acknowledge(&mut self, block: i32, time: i32) {
        if block != self.client_block {
            self.bindings.drop_client("broken download");
            return;
        }
        if self.block_sizes[(self.client_block & 7) as usize] == 0 {
            self.close();
            return;
        }
        self.send_time = time;
        self.client_block = self.client_block.wrapping_add(1);
    }

    fn open(&mut self, writer: &mut Q3MsgWriter) -> Result<bool, Q3NetError> {
        let stock = q3_stock_package(&self.name);
        let enabled = self.bindings.enabled();
        if enabled && stock.is_none() {
            check_q3_download_name(&self.name)?;
            let mut file = self.bindings.open(&self.name);
            self.size = file.as_ref().map_or(-1, |file| file.size());
            self.file = file.take();
            if self.size > 0 {
                self.current_block = 0;
                self.client_block = 0;
                self.xmit_block = 0;
                self.count = 0;
                self.eof = false;
                return Ok(true);
            }
        }
        let error = if let Some(stock) = stock {
            if stock == "missionpack" {
                format!(
                    "Cannot autodownload Team Arena file \"{}\"\nThe Team Arena mission pack can be found in your local game store.",
                    self.name
                )
            } else {
                format!("Cannot autodownload id pk3 file \"{}\"", self.name)
            }
        } else if !enabled {
            format!(
                "Could not download \"{}\" because autodownloading is disabled on the server.\n\n{}",
                self.name,
                if self.bindings.pure() {
                    "You will need to get this file elsewhere before you can connect to this pure server.\n"
                } else {
                    "The server you are connecting to is not a pure server, set autodownload to No in your settings and you might be able to join the game anyway.\n"
                }
            )
        } else {
            format!("File \"{}\" not found on server for autodownloading.\n", self.name)
        };
        writer.write_byte(ServerOpcode::Download as i32)?;
        writer.write_short(0)?;
        writer.write_long(-1)?;
        writer.write_string(Some(&error.chars().take(1023).collect::<String>()))?;
        self.close();
        Ok(false)
    }

    /// Write due blocks.
    pub fn write(
        &mut self,
        writer: &mut Q3MsgWriter,
        time: i32,
        settings: Q3DownloadRate,
    ) -> Result<(), Q3NetError> {
        if self.name.is_empty() || (self.file.is_none() && !self.open(writer)?) {
            return Ok(());
        }
        if self.file.is_none() {
            return Err(Q3NetError::Protocol("Opened Q3 download has no file"));
        }
        while self.current_block - self.client_block < 8 && self.size != self.count {
            let index = (self.current_block & 7) as usize;
            let Some(buffer) = self.blocks.get_mut(index) else {
                return Err(Q3NetError::Range("Missing download block storage"));
            };
            let bytes = self.file.as_mut().expect("checked").read(buffer);
            if bytes == 0 || bytes > buffer.len() || self.count + bytes as i64 > self.size {
                self.close();
                return Err(Q3NetError::Protocol(
                    "Source download file changed or returned an invalid read",
                ));
            }
            self.block_sizes[index] = bytes as i32;
            self.count = self.count.wrapping_add(bytes as i64);
            self.current_block = self.current_block.wrapping_add(1);
        }
        if self.count == self.size && !self.eof && self.current_block - self.client_block < 8 {
            self.block_sizes[(self.current_block & 7) as usize] = 0;
            self.current_block = self.current_block.wrapping_add(1);
            self.eof = true;
        }
        let mut rate = settings.rate;
        if settings.max_rate != 0 {
            rate = rate.min(settings.max_rate.max(1000));
        }
        let mut blocks = if rate == 0 {
            1
        } else {
            (rate
                .wrapping_mul(settings.snapshot_msec)
                .wrapping_div(1000)
                .wrapping_add(2048))
            .wrapping_div(2048)
        };
        if blocks < 0 {
            blocks = 1;
        }
        while blocks > 0 {
            blocks -= 1;
            if self.client_block == self.current_block {
                return Ok(());
            }
            if self.xmit_block == self.current_block {
                if time.wrapping_sub(self.send_time) > 1000 {
                    self.xmit_block = self.client_block;
                } else {
                    return Ok(());
                }
            }
            let index = (self.xmit_block & 7) as usize;
            let size = self.block_sizes[index];
            writer.write_byte(ServerOpcode::Download as i32)?;
            writer.write_short(self.xmit_block)?;
            if self.xmit_block == 0 {
                writer.write_long(self.size as i32)?;
            }
            writer.write_short(size)?;
            if size != 0 {
                let end = size as usize;
                writer.write_data(&self.blocks[index][..end])?;
            }
            self.xmit_block = self.xmit_block.wrapping_add(1);
            self.send_time = time;
        }
        Ok(())
    }
}

/// Client download file (`Q3DownloadWriteFile`).
pub trait Q3DownloadWriteFile {
    /// Write bytes.
    fn write_bytes(&mut self, bytes: &[u8]);
    /// Close the file.
    fn close(&mut self);
}

/// Client download bindings (`Q3DownloadClientBindings`).
pub trait Q3DownloadClientBindings {
    /// Assert the current snapshot is current.
    fn assert_current(&mut self);
    /// Open a temporary file (exclusive creation, containment, no replace).
    fn open_temporary(&mut self, path: &str) -> Option<Box<dyn Q3DownloadWriteFile>>;
    /// Publish the temporary file.
    fn publish_temporary(&mut self, temporary: &str, destination: &str);
    /// Queue a reliable command.
    fn reliable(&mut self, text: &str);
    /// Send a packet.
    fn send_packet(&mut self);
    /// Report progress.
    fn progress(&mut self, name: &str, count: i32, size: i32);
    /// Finish the download queue.
    fn completed(&mut self);
}

/// Client download state (`Q3ClientDownload`).
pub struct Q3ClientDownload<'a> {
    bindings: &'a mut dyn Q3DownloadClientBindings,
    file: Option<Box<dyn Q3DownloadWriteFile>>,
    name: String,
    temporary: String,
    block: i32,
    count: i32,
    size: i32,
}

impl<'a> Q3ClientDownload<'a> {
    /// Build a download over bindings.
    pub fn new(bindings: &'a mut dyn Q3DownloadClientBindings) -> Self {
        Self { bindings, file: None, name: String::new(), temporary: String::new(), block: 0, count: 0, size: 0 }
    }

    /// Begin downloading a remote file to a local name.
    pub fn begin(&mut self, remote: &str, local: &str) -> Result<(), Q3NetError> {
        self.bindings.assert_current();
        check_q3_download_name(remote)?;
        check_q3_download_name(local)?;
        self.close();
        self.name = local.to_string();
        self.temporary = format!("{local}.tmp");
        self.block = 0;
        self.count = 0;
        self.size = 0;
        self.bindings.progress(remote, 0, 0);
        self.bindings.reliable(&format!("download {remote}"));
        Ok(())
    }

    /// Publish the advertised size.
    pub fn publish_size(&mut self, size: i32) -> i32 {
        self.size = size;
        self.bindings.progress(&self.name.clone(), self.count, size);
        size
    }

    /// Receive one download block.
    pub fn receive(&mut self, download: &DownloadBlock) -> Result<(), Q3NetError> {
        self.bindings.assert_current();
        if let DownloadBlock::Error { message, .. } = download {
            return Err(Q3NetError::drop("drop", message.clone()));
        }
        let block = match download {
            DownloadBlock::Start { .. } => 0,
            DownloadBlock::Chunk { number, .. } => *number,
            DownloadBlock::Error { .. } => unreachable!("checked"),
        };
        if block != self.block {
            return Ok(());
        }
        if self.file.is_none() {
            if self.temporary.is_empty() {
                self.bindings.reliable("stopdl");
                return Ok(());
            }
            let temporary = self.temporary.clone();
            self.file = self.bindings.open_temporary(&temporary);
            self.bindings.assert_current();
            if self.file.is_none() {
                self.bindings.reliable("stopdl");
                self.bindings.completed();
                return Ok(());
            }
        }
        let data = match download {
            DownloadBlock::Start { data, .. } | DownloadBlock::Chunk { data, .. } => data,
            DownloadBlock::Error { .. } => unreachable!("checked"),
        };
        if !data.is_empty() {
            self.file.as_mut().expect("opened").write_bytes(data);
            self.bindings.assert_current();
        }
        let block = self.block;
        self.bindings.reliable(&format!("nextdl {block}"));
        self.block = self.block.wrapping_add(1);
        self.count = self.count.wrapping_add(data.len() as i32);
        let name = self.name.clone();
        let (count, size) = (self.count, self.size);
        self.bindings.progress(&name, count, size);
        if data.is_empty() {
            if let Some(mut file) = self.file.take() {
                file.close();
            }
            self.bindings.assert_current();
            let (temporary, name) = (self.temporary.clone(), self.name.clone());
            self.bindings.publish_temporary(&temporary, &name);
            self.bindings.assert_current();
            self.name.clear();
            self.temporary.clear();
            let (count, size) = (self.count, self.size);
            self.bindings.progress("", count, size);
            self.bindings.send_packet();
            self.bindings.assert_current();
            self.bindings.send_packet();
            self.bindings.assert_current();
            self.bindings.completed();
        }
        Ok(())
    }

    /// Close the current download.
    pub fn close(&mut self) {
        if let Some(mut file) = self.file.take() {
            file.close();
        }
        self.name.clear();
        self.temporary.clear();
    }
}

/// Client opcode (`ClientOpcode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ClientOpcode {
    /// No-op.
    Nop = 1,
    /// Move.
    Move = 2,
    /// Move without delta.
    MoveNoDelta = 3,
    /// Reliable command.
    Command = 4,
    /// End.
    Eof = 5,
}

/// Maximum user commands per packet (`MAX_PACKET_USER_COMMANDS`).
pub const MAX_PACKET_USER_COMMANDS: usize = 32;

/// Client header (`ClientHeader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientHeader {
    /// Server id.
    pub server_id: i32,
    /// Message acknowledge.
    pub message_acknowledge: i32,
    /// Reliable acknowledge.
    pub reliable_acknowledge: i32,
}

/// Client movement (`ClientMovement`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMovement {
    /// Kind.
    pub kind: ClientMovementKind,
    /// Commands in generation order.
    pub commands: Vec<WireUserCommand>,
}

/// Client movement kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMovementKind {
    /// Delta against the acknowledged message.
    Move,
    /// No delta.
    MoveNoDelta,
}

/// Client message (`ClientMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMessage {
    /// Header.
    pub header: ClientHeader,
    /// Reliable commands.
    pub commands: Vec<ReliableCommand>,
    /// Movement.
    pub movement: Option<ClientMovement>,
}

/// Key context (`ClientKeyContext`).
pub struct ClientKeyContext<'a> {
    /// Checksum feed.
    pub checksum_feed: i32,
    /// Server command lookup.
    pub server_command: &'a dyn Fn(i32) -> String,
}

/// Decode context (`ClientDecodeContext`).
pub struct ClientDecodeContext<'a> {
    /// Checksum feed.
    pub checksum_feed: i32,
    /// Server command lookup.
    pub server_command: &'a dyn Fn(i32) -> String,
    /// Reliable sequence.
    pub reliable_sequence: i32,
    /// Last client command.
    pub last_client_command: i32,
    /// Last user command time.
    pub last_user_command_time: i32,
}

/// Decoded client movement (`DecodedClientMovement`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedClientMovement {
    /// Kind.
    pub kind: ClientMovementKind,
    /// Commands.
    pub commands: Vec<WireUserCommand>,
    /// Delta message.
    pub delta_message: i32,
    /// Executable commands.
    pub executable_commands: Vec<WireUserCommand>,
    /// Last user command time.
    pub last_user_command_time: i32,
}

/// Unfiltered client movement (`UnfilteredClientMovement`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnfilteredClientMovement {
    /// Kind.
    pub kind: ClientMovementKind,
    /// Commands (at least one).
    pub commands: Vec<WireUserCommand>,
    /// Delta message.
    pub delta_message: i32,
}

/// Client message part (`ClientMessagePart`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessagePart {
    /// Reliable command.
    Command(ReliableCommand),
    /// Movement follows.
    Movement {
        /// Kind.
        movement_kind: ClientMovementKind,
        /// Delta message.
        delta_message: i32,
    },
    /// End.
    Eof,
}

#[derive(Debug, Clone, Copy)]
enum ClientReadPhase {
    Header,
    Commands(ClientHeader),
    Movement(ClientHeader, ClientMovementKind),
    Terminal,
    Done,
    Failed,
}

/// Decoded client message (`DecodedClientMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedClientMessage {
    /// Accepted.
    Accepted {
        /// Header.
        header: ClientHeader,
        /// Commands.
        commands: Vec<ReliableCommand>,
        /// Last client command.
        last_client_command: i32,
        /// Movement.
        movement: Option<DecodedClientMovement>,
    },
    /// Rejected.
    Rejected {
        /// Header.
        header: ClientHeader,
        /// Commands.
        commands: Vec<ReliableCommand>,
        /// Last client command.
        last_client_command: i32,
        /// Reason.
        reason: ClientReject,
    },
}

/// Client rejection reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientReject {
    /// Negative message acknowledge.
    NegativeMessageAcknowledge,
    /// Negative reliable acknowledge.
    NegativeReliableAcknowledge,
    /// Stale reliable acknowledge.
    StaleReliableAcknowledge,
    /// Future reliable acknowledge.
    FutureReliableAcknowledge,
    /// Lost reliable command.
    LostReliableCommand,
}

/// Source `Com_HashKey` with signed char bytes (`commandHash`).
pub fn command_hash(text: &str, max_length: usize) -> Result<i32, Q3NetError> {
    let mut hash: i32 = 0;
    let chars: Vec<char> = text.chars().collect();
    for (index, ch) in chars.iter().enumerate().take(max_length) {
        let byte = *ch as u32;
        if byte == 0 {
            break;
        }
        if byte > 255 {
            return Err(Q3NetError::Range("Command hash requires an engine byte string"));
        }
        let signed = (byte as u8) as i8 as i32;
        hash = hash.wrapping_add(signed.wrapping_mul(119 + index as i32));
    }
    Ok(hash ^ (hash >> 10) ^ (hash >> 20))
}

fn move_key(header: ClientHeader, checksum_feed: i32, server_command: &dyn Fn(i32) -> String) -> Result<i32, Q3NetError> {
    Ok(checksum_feed ^ header.message_acknowledge ^ command_hash(&server_command(header.reliable_acknowledge), 32)?)
}

/// Begin a client message (`beginClientMessage`).
pub fn begin_client_message(
    header: ClientHeader,
    commands: &[ReliableCommand],
) -> Result<Q3MsgWriter, Q3NetError> {
    for acknowledge in [header.message_acknowledge, header.reliable_acknowledge] {
        if acknowledge < 0 {
            return Err(Q3NetError::Range("Client acknowledgements must be nonnegative int32 values"));
        }
    }
    let mut writer = Q3MsgWriter::new(MessageMode::Bitstream, MAX_MESSAGE_LENGTH)?;
    writer.write_long(header.server_id)?;
    writer.write_long(header.message_acknowledge)?;
    writer.write_long(header.reliable_acknowledge)?;
    let mut previous: Option<i32> = None;
    for command in commands {
        if previous.is_some_and(|previous| command.sequence <= previous) {
            return Err(Q3NetError::Range(
                "Client reliable commands must have increasing int32 sequences",
            ));
        }
        writer.write_byte(ClientOpcode::Command as i32)?;
        writer.write_long(command.sequence)?;
        writer.write_string(Some(&command.text))?;
        previous = Some(command.sequence);
    }
    Ok(writer)
}

/// Write client movement (`writeClientMovement`).
pub fn write_client_movement(
    writer: &mut Q3MsgWriter,
    movement: &ClientMovement,
    header: ClientHeader,
    context: &ClientKeyContext<'_>,
) -> Result<(), Q3NetError> {
    if movement.commands.is_empty() || movement.commands.len() > MAX_PACKET_USER_COMMANDS {
        return Err(Q3NetError::Range("Movement needs 1 through 32 backup commands"));
    }
    writer.write_byte(
        if movement.kind == ClientMovementKind::Move {
            ClientOpcode::Move as i32
        } else {
            ClientOpcode::MoveNoDelta as i32
        },
    )?;
    writer.write_byte(movement.commands.len() as i32)?;
    let key = move_key(header, context.checksum_feed, context.server_command)?;
    let mut old = WireUserCommand::default();
    for command in &movement.commands {
        write_delta_user_command(writer, &old, command, Some(key))?;
        old = command.clone();
    }
    Ok(())
}

/// Finish a client message (`finishClientMessage`).
pub fn finish_client_message(writer: &mut Q3MsgWriter) -> Result<Vec<u8>, Q3NetError> {
    writer.write_byte(ClientOpcode::Eof as i32)?;
    if writer.overflowed() {
        return Err(Q3NetError::Range("Client message exceeds MAX_MSGLEN"));
    }
    Ok(writer.to_bytes().to_vec())
}

/// Encode a full client message (`encodeClientMessage`).
pub fn encode_client_message(
    message: &ClientMessage,
    context: &ClientKeyContext<'_>,
) -> Result<Vec<u8>, Q3NetError> {
    let mut writer = begin_client_message(message.header, &message.commands)?;
    if let Some(movement) = &message.movement {
        write_client_movement(&mut writer, movement, message.header, context)?;
    }
    finish_client_message(&mut writer)
}

/// Incremental client-message decoder (`ClientMessageReader`).
pub struct ClientMessageReader<'a> {
    reader: Q3MsgReader<'a>,
    /// Server id and message acknowledge, read eagerly.
    pub prefix: ClientHeaderPrefix,
    phase: ClientReadPhase,
}

/// Eager client prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientHeaderPrefix {
    /// Server id.
    pub server_id: i32,
    /// Message acknowledge.
    pub message_acknowledge: i32,
}

impl<'a> ClientMessageReader<'a> {
    /// Build a reader over a borrowed datagram.
    pub fn new(bytes: &'a [u8]) -> Result<Self, Q3NetError> {
        let mut reader = Q3MsgReader::new(bytes, MessageMode::Bitstream)?;
        let prefix = ClientHeaderPrefix {
            server_id: reader.read_long()?,
            message_acknowledge: reader.read_long()?,
        };
        Ok(Self { reader, prefix, phase: ClientReadPhase::Header })
    }

    /// Read count.
    #[must_use]
    pub fn read_count(&self) -> usize {
        self.reader.read_count()
    }

    /// Read the header after accepting the prefix.
    pub fn read_header(&mut self) -> Result<ClientHeader, Q3NetError> {
        if !matches!(self.phase, ClientReadPhase::Header) {
            return Err(Q3NetError::Protocol("Cannot read client header during phase"));
        }
        self.phase = ClientReadPhase::Failed;
        let header = ClientHeader {
            server_id: self.prefix.server_id,
            message_acknowledge: self.prefix.message_acknowledge,
            reliable_acknowledge: self.reader.read_long()?,
        };
        self.phase = ClientReadPhase::Commands(header);
        Ok(header)
    }

    /// Next part; a returned command must be admitted before continuing.
    pub fn next(&mut self) -> Result<ClientMessagePart, Q3NetError> {
        let ClientReadPhase::Commands(header) = self.phase else {
            return Err(Q3NetError::Protocol("Cannot read next client part during phase"));
        };
        self.phase = ClientReadPhase::Failed;
        let opcode = self.reader.read_byte()?;
        if opcode == ClientOpcode::Command as i32 {
            let command = ReliableCommand {
                sequence: self.reader.read_long()?,
                text: self.reader.read_string()?,
            };
            self.phase = ClientReadPhase::Commands(header);
            return Ok(ClientMessagePart::Command(command));
        }
        if opcode == ClientOpcode::Eof as i32 {
            self.phase = ClientReadPhase::Done;
            return Ok(ClientMessagePart::Eof);
        }
        if opcode != ClientOpcode::Move as i32 && opcode != ClientOpcode::MoveNoDelta as i32 {
            return Err(Q3NetError::ClientOpcode { opcode });
        }
        let kind = if opcode == ClientOpcode::Move as i32 {
            ClientMovementKind::Move
        } else {
            ClientMovementKind::MoveNoDelta
        };
        self.phase = ClientReadPhase::Movement(header, kind);
        Ok(ClientMessagePart::Movement {
            movement_kind: kind,
            delta_message: if kind == ClientMovementKind::Move {
                header.message_acknowledge
            } else {
                -1
            },
        })
    }

    /// Decode backups without time filtering or trailing-byte reads.
    pub fn read_movement(
        &mut self,
        context: &ClientKeyContext<'_>,
    ) -> Result<UnfilteredClientMovement, Q3NetError> {
        let ClientReadPhase::Movement(header, kind) = self.phase else {
            return Err(Q3NetError::Protocol("Cannot read client movement during phase"));
        };
        self.phase = ClientReadPhase::Failed;
        let count = self.reader.read_byte()?;
        if count < 1 || count as usize > MAX_PACKET_USER_COMMANDS {
            return Err(Q3NetError::ClientCommandCount { count });
        }
        let key = move_key(header, context.checksum_feed, context.server_command)?;
        let mut old =
            read_delta_user_command(&mut self.reader, &WireUserCommand::default(), Some(key))?;
        let mut commands = vec![old.clone()];
        for _ in 1..count {
            old = read_delta_user_command(&mut self.reader, &old, Some(key))?;
            commands.push(old.clone());
        }
        self.phase = ClientReadPhase::Terminal;
        Ok(UnfilteredClientMovement {
            kind,
            commands,
            delta_message: if kind == ClientMovementKind::Move {
                header.message_acknowledge
            } else {
                -1
            },
        })
    }

    /// Validate the terminal EOF.
    pub fn validate_terminal(&mut self) -> Result<(), Q3NetError> {
        if !matches!(self.phase, ClientReadPhase::Terminal) {
            return Err(Q3NetError::Protocol("Cannot validate client terminal during phase"));
        }
        self.phase = ClientReadPhase::Failed;
        if self.reader.read_byte()? != ClientOpcode::Eof as i32 {
            return Err(Q3NetError::Protocol("Missing terminal clc_EOF after movement"));
        }
        self.phase = ClientReadPhase::Done;
        Ok(())
    }
}

/// Filter movement against the last user command time (`filterClientMovement`).
pub fn filter_client_movement(
    movement: &UnfilteredClientMovement,
    last_user_command_time: i32,
) -> Result<DecodedClientMovement, Q3NetError> {
    let Some(latest) = movement.commands.last() else {
        return Err(Q3NetError::Range("Movement needs 1 through 32 backup commands"));
    };
    if movement.commands.len() > MAX_PACKET_USER_COMMANDS {
        return Err(Q3NetError::Range("Movement needs 1 through 32 backup commands"));
    }
    let mut executable = Vec::new();
    let mut time = last_user_command_time;
    for command in &movement.commands {
        if command.server_time > latest.server_time || command.server_time <= time {
            continue;
        }
        executable.push(command.clone());
        time = command.server_time;
    }
    Ok(DecodedClientMovement {
        kind: movement.kind,
        commands: movement.commands.clone(),
        delta_message: movement.delta_message,
        executable_commands: executable,
        last_user_command_time: time,
    })
}

/// Decode a whole client message (`decodeClientMessage`).
pub fn decode_client_message(
    bytes: &[u8],
    context: &ClientDecodeContext<'_>,
) -> Result<DecodedClientMessage, Q3NetError> {
    let mut reader = ClientMessageReader::new(bytes)?;
    let header = reader.read_header()?;
    let mut commands = Vec::new();
    let mut last_client_command = context.last_client_command;
    if header.message_acknowledge < 0 {
        return Ok(DecodedClientMessage::Rejected {
            header,
            commands,
            last_client_command,
            reason: ClientReject::NegativeMessageAcknowledge,
        });
    }
    if header.reliable_acknowledge < 0 {
        return Ok(DecodedClientMessage::Rejected {
            header,
            commands,
            last_client_command,
            reason: ClientReject::NegativeReliableAcknowledge,
        });
    }
    if header.reliable_acknowledge < context.reliable_sequence - 64 {
        return Ok(DecodedClientMessage::Rejected {
            header: ClientHeader {
                reliable_acknowledge: context.reliable_sequence,
                ..header
            },
            commands,
            last_client_command,
            reason: ClientReject::StaleReliableAcknowledge,
        });
    }
    if header.reliable_acknowledge > context.reliable_sequence {
        return Ok(DecodedClientMessage::Rejected {
            header,
            commands,
            last_client_command,
            reason: ClientReject::FutureReliableAcknowledge,
        });
    }
    loop {
        match reader.next()? {
            ClientMessagePart::Command(command) => {
                if command.sequence > last_client_command {
                    if command.sequence > last_client_command + 1 {
                        return Ok(DecodedClientMessage::Rejected {
                            header,
                            commands,
                            last_client_command,
                            reason: ClientReject::LostReliableCommand,
                        });
                    }
                    commands.push(command.clone());
                    last_client_command = command.sequence;
                }
            }
            ClientMessagePart::Eof => {
                return Ok(DecodedClientMessage::Accepted {
                    header,
                    commands,
                    last_client_command,
                    movement: None,
                });
            }
            ClientMessagePart::Movement { .. } => {
                let key_context = ClientKeyContext {
                    checksum_feed: context.checksum_feed,
                    server_command: context.server_command,
                };
                let movement = reader.read_movement(&key_context)?;
                reader.validate_terminal()?;
                return Ok(DecodedClientMessage::Accepted {
                    header,
                    commands,
                    last_client_command,
                    movement: Some(filter_client_movement(
                        &movement,
                        context.last_user_command_time,
                    )?),
                });
            }
        }
    }
}

/// Shared circular entity storage (`Q3SnapshotEntities`).
#[derive(Debug, Clone)]
pub struct Q3SnapshotEntities {
    cells: Vec<Option<Q3EntityState>>,
    /// Next absolute slot.
    pub next: i32,
    /// Storage length.
    pub length: usize,
}

impl Q3SnapshotEntities {
    /// Build storage.
    pub fn new(length: usize) -> Result<Self, Q3NetError> {
        if length < 1 || length > 131072 {
            return Err(Q3NetError::Range("Q3 snapshot entity storage outside 1..131072"));
        }
        Ok(Self { cells: vec![None; length], next: 0, length })
    }

    /// Append an entity.
    pub fn append(&mut self, entity: &Q3EntityState) -> Result<(), Q3NetError> {
        let index = (self.next as usize) % self.length;
        let Some(cell) = self.cells.get_mut(index) else {
            return Err(Q3NetError::Range("Invalid snapshot entity slot"));
        };
        *cell = Some(entity.clone());
        self.next = self.next.wrapping_add(1);
        if self.next >= 0x7fff_fffe {
            return Err(Q3NetError::drop("fatal", "svs.nextSnapshotEntities wrapped"));
        }
        Ok(())
    }

    /// Read an absolute slot (missing cells decode as zero).
    pub fn read(&self, absolute: i32) -> Result<Q3EntityState, Q3NetError> {
        if absolute < 0 {
            return Err(Q3NetError::Range("Invalid snapshot entity position"));
        }
        let Some(cell) = self.cells.get((absolute as usize) % self.length) else {
            return Err(Q3NetError::Range("Invalid snapshot entity position"));
        };
        Ok(cell.clone().unwrap_or_default())
    }
}

/// Server frame (`Q3ServerFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ServerFrame {
    /// Player state.
    pub player_state: Q3PlayerState,
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// First entity.
    pub first_entity: i32,
    /// Entity count.
    pub num_entities: usize,
    /// Message size.
    pub message_size: usize,
    /// Message sent.
    pub message_sent: i32,
    /// Message acknowledged.
    pub message_acked: i32,
}

/// Server snapshot history (`Q3ServerSnapshotHistory`).
pub struct Q3ServerSnapshotHistory<'a> {
    /// Shared entity storage.
    pub entities: Q3SnapshotEntities,
    /// Product.
    pub product: Q3Product,
    /// Baseline lookup.
    pub baseline: &'a dyn Fn(i32) -> Q3EntityState,
    frames: Vec<Q3ServerFrame>,
}

impl<'a> Q3ServerSnapshotHistory<'a> {
    /// Build history over shared storage.
    #[must_use]
    pub fn new(
        entities: Q3SnapshotEntities,
        product: Q3Product,
        baseline: &'a dyn Fn(i32) -> Q3EntityState,
    ) -> Self {
        let frames = (0..32)
            .map(|_| Q3ServerFrame {
                player_state: Q3PlayerState::new(product),
                area_mask: Vec::new(),
                first_entity: 0,
                num_entities: 0,
                message_size: 0,
                message_sent: 0,
                message_acked: 0,
            })
            .collect();
        Self { entities, product, baseline, frames }
    }

    /// Borrow a frame slot.
    pub fn frame(&self, sequence: i32) -> Result<&Q3ServerFrame, Q3NetError> {
        self.frames
            .get((sequence & 31) as usize)
            .ok_or(Q3NetError::Range("Missing source frame ring slot"))
    }

    /// Mutably borrow a frame slot.
    pub fn frame_mut(&mut self, sequence: i32) -> Result<&mut Q3ServerFrame, Q3NetError> {
        self.frames
            .get_mut((sequence & 31) as usize)
            .ok_or(Q3NetError::Range("Missing source frame ring slot"))
    }

    /// Capture a frame.
    pub fn capture(
        &mut self,
        sequence: i32,
        player: &Q3PlayerState,
        area_mask: &[u8],
        entities: &[Q3EntityState],
    ) -> Result<(), Q3NetError> {
        if area_mask.len() > 32 || entities.len() > 256 {
            return Err(Q3NetError::Range("Q3 snapshot exceeds source area/entity capacities"));
        }
        let index = (sequence & 31) as usize;
        if index >= self.frames.len() {
            return Err(Q3NetError::Range("Missing source frame ring slot"));
        }
        self.frames[index].player_state = player.clone();
        self.frames[index].area_mask = area_mask.to_vec();
        self.frames[index].first_entity = self.entities.next;
        self.frames[index].num_entities = 0;
        for entity in entities {
            self.entities.append(entity)?;
            self.frames[index].num_entities += 1;
        }
        Ok(())
    }

    /// Resolve the delta message.
    pub fn delta(&self, sequence: i32, requested: i32, active: bool) -> Result<i32, Q3NetError> {
        if requested <= 0 || !active || sequence - requested >= 29 {
            return Ok(-1);
        }
        let old = self.frame(requested)?;
        if old.first_entity <= self.entities.next - self.entities.length as i32 {
            return Ok(-1);
        }
        Ok(requested)
    }

    /// Write a snapshot from the physical ring, including wrap effects.
    pub fn write(
        &self,
        writer: &mut Q3MsgWriter,
        sequence: i32,
        requested: i32,
        active: bool,
        server_time: i32,
        flags: i32,
    ) -> Result<(), Q3NetError> {
        let current = self.frame(sequence)?.clone();
        let delta = self.delta(sequence, requested, active)?;
        let old = if delta < 0 { None } else { Some(self.frame(delta)?.clone()) };
        writer.write_byte(ServerOpcode::Snapshot as i32)?;
        writer.write_long(server_time)?;
        writer.write_byte(if delta < 0 { 0 } else { sequence - delta })?;
        writer.write_byte(flags)?;
        writer.write_byte(current.area_mask.len() as i32)?;
        writer.write_data(&current.area_mask)?;
        write_delta_player_state(
            writer,
            old.as_ref().map(|old| &old.player_state),
            &current.player_state,
        )?;
        let mut new_index = 0;
        let mut old_index = 0;
        while new_index < current.num_entities
            || old.as_ref().is_some_and(|old| old_index < old.num_entities)
        {
            let next = if new_index < current.num_entities {
                Some(self.entities.read(current.first_entity + new_index as i32)?)
            } else {
                None
            };
            let previous = if old.as_ref().is_some_and(|old| old_index < old.num_entities) {
                let old = old.as_ref().expect("checked");
                Some(self.entities.read(old.first_entity + old_index as i32)?)
            } else {
                None
            };
            match (next, previous) {
                (Some(next), Some(previous)) if next.number == previous.number => {
                    write_delta_entity(writer, Some(&previous), Some(&next), false)?;
                    new_index += 1;
                    old_index += 1;
                }
                (Some(next), previous)
                    if previous.as_ref().is_none_or(|previous| next.number < previous.number) =>
                {
                    write_delta_entity(writer, Some(&(self.baseline)(next.number)), Some(&next), true)?;
                    new_index += 1;
                }
                (_, Some(previous)) => {
                    write_delta_entity(writer, Some(&previous), None, true)?;
                    old_index += 1;
                }
                _ => return Err(Q3NetError::Range("Missing snapshot merge entity")),
            }
        }
        writer.write_bits(1023, 10)?;
        Ok(())
    }

    /// Copy a frame into a snapshot.
    pub fn copy(
        &self,
        sequence: i32,
        delta_number: i32,
        server_time: i32,
        flags: i32,
        server_command_number: i32,
    ) -> Result<Snapshot, Q3NetError> {
        let frame = self.frame(sequence)?;
        let mut entities = Vec::with_capacity(frame.num_entities);
        for index in 0..frame.num_entities {
            entities.push(self.entities.read(frame.first_entity + index as i32)?);
        }
        Ok(Snapshot {
            message_number: sequence,
            delta_number,
            server_time,
            flags,
            server_command_number,
            parse_entities_number: frame.first_entity,
            area_mask: frame.area_mask.clone(),
            player_state: frame.player_state.clone(),
            entities,
        })
    }
}

/// Server command bindings (`Q3ServerCommandBindings`).
pub trait Q3ServerCommandBindings {
    /// Assert the current snapshot is current.
    fn assert_current(&mut self);
    /// Apply system info.
    fn system_info(&mut self) -> Result<(), Q3NetError>;
    /// Restart the map.
    fn map_restart(&mut self);
    /// Whether a local server runs.
    fn local_server_running(&self) -> bool;
    /// Take a level shot.
    fn level_shot(&mut self);
    /// Game state storage; the donor passes the same object as executor
    /// state and closes over it in bindings, which Rust expresses as one host.
    fn game_state(&mut self) -> &mut ClientGameStateStorage;
}

/// Server command executor (`Q3ServerCommandExecutor`).
#[derive(Debug, Clone, Default)]
pub struct Q3ServerCommandExecutor {
    big_config_string: String,
}

impl Q3ServerCommandExecutor {
    /// Fresh executor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the big-configstring accumulator.
    pub fn clear(&mut self) {
        self.big_config_string.clear();
    }

    /// Execute one server command, returning tokens or `None` when buffering.
    pub fn execute(
        &mut self,
        command: &str,
        host: &mut dyn Q3ServerCommandBindings,
    ) -> Result<Option<Vec<String>>, Q3NetError> {
        let mut text = command.to_string();
        let mut argv = tokenize_command(&text, Dialect::Q3, TextMode::Source)?.argv;
        let mut name = argv.first().cloned().unwrap_or_default();
        if name == "disconnect" {
            if argv.len() >= 2 {
                return Err(Q3NetError::drop(
                    "server-disconnect",
                    format!("Server Disconnected - {}", argv[1]),
                ));
            }
            return Err(Q3NetError::drop("server-disconnect", "Server disconnected\n"));
        }
        if name == "bcs0" {
            let assembled =
                format!("cs {} \"{}", argv.get(1).cloned().unwrap_or_default(), argv.get(2).cloned().unwrap_or_default());
            self.big_config_string = assembled.chars().take(8191).collect();
            return Ok(None);
        }
        if name == "bcs1" || name == "bcs2" {
            let suffix = argv.get(2).cloned().unwrap_or_default();
            let last = name == "bcs2";
            if self.big_config_string.len() + suffix.len() + usize::from(last) >= 8192 {
                return Err(Q3NetError::drop("drop", "bcs exceeded BIG_INFO_STRING"));
            }
            self.big_config_string.push_str(&suffix);
            if !last {
                return Ok(None);
            }
            self.big_config_string.push('"');
            text = self.big_config_string.clone();
            argv = tokenize_command(&text, Dialect::Q3, TextMode::Source)?.argv;
            name = argv.first().cloned().unwrap_or_default();
        }
        if name == "cs" {
            let index = native_atoi(argv.get(1).map(String::as_str).unwrap_or(""))?;
            let value = argv.get(2..).unwrap_or(&[]).join(" ");
            if index < 0 || index as usize >= MAX_CONFIGSTRINGS {
                return Err(Q3NetError::drop("drop", "configstring > MAX_CONFIGSTRINGS"));
            }
            let changed = host.game_state().modify(index as usize, &value)?;
            if changed && index == 1 {
                host.system_info()?;
                host.assert_current();
            }
            argv = tokenize_command(&text, Dialect::Q3, TextMode::Source)?.argv;
        } else if name == "map_restart" {
            host.map_restart();
        } else if name == "clientLevelShot" {
            if !host.local_server_running() {
                return Ok(None);
            }
            host.level_shot();
        }
        Ok(Some(argv))
    }
}

/// Rcon output sink inside `redirect`.
pub trait Q3RconSink {
    /// Emit redirected output.
    fn emit(&mut self, text: &str);
    /// Execute a command with output captured.
    fn execute(&mut self, command: &str);
}

/// Rcon bindings (`Q3RconBindings`).
pub trait Q3RconBindings {
    /// Rcon password.
    fn password(&self) -> String;
    /// Print outside the redirect.
    fn print(&mut self, text: &str);
    /// Run `produce` with output captured, then flush through `send`.
    fn redirect(
        &mut self,
        to: &NetworkAddress,
        capacity: usize,
        produce: &mut dyn FnMut(&mut dyn Q3RconSink),
    );
    /// Execute a command.
    fn execute(&mut self, command: &str);
    /// Send bytes.
    fn send(&mut self, to: &NetworkAddress, bytes: &[u8]);
}

/// Rcon throttle state (`Q3RconState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Q3RconState {
    /// Last handled time.
    pub last_time: u32,
}

/// Remote console (`Q3Rcon`).
pub struct Q3Rcon<'a> {
    /// Throttle state (kept across servers/maps like source statics).
    pub state: Q3RconState,
    bindings: &'a mut dyn Q3RconBindings,
}

impl<'a> Q3Rcon<'a> {
    /// Build remote console over bindings.
    pub fn new(state: Q3RconState, bindings: &'a mut dyn Q3RconBindings) -> Self {
        Self { state, bindings }
    }

    /// Handle one rcon packet.
    pub fn receive(
        &mut self,
        from: &NetworkAddress,
        packet: &ConnectionlessPacket,
        milliseconds: u64,
    ) -> Result<(), Q3NetError> {
        let time = milliseconds as u32;
        if time < self.state.last_time.wrapping_add(500) {
            return Ok(());
        }
        self.state.last_time = time;
        let password = source_command_text(&self.bindings.password())?;
        let valid = !password.is_empty()
            && password == *packet.arguments.first().cloned().unwrap_or_default();
        let empty = password.is_empty();
        let line = packet.line.clone();
        self.bindings.redirect(from, 1008, &mut |sink| {
            if empty {
                sink.emit("No rconpassword set on the server.\n");
                return;
            }
            if !valid {
                sink.emit("Bad rconpassword.\n");
                return;
            }
            let chars: Vec<char> = line.chars().collect();
            let mut cursor = 4;
            while chars.get(cursor) == Some(&' ') {
                cursor += 1;
            }
            while cursor < chars.len() && chars[cursor] != ' ' {
                cursor += 1;
            }
            while chars.get(cursor) == Some(&' ') {
                cursor += 1;
            }
            let command: String = chars[cursor..].iter().take(1023).collect();
            if !command.is_empty() {
                sink.execute(&command);
            }
        });
        Ok(())
    }
}

/// Encode an rcon request (`encodeQ3Rcon`).
pub fn encode_q3_rcon(password: &str, command: &str) -> Result<Vec<u8>, Q3NetError> {
    encode_connectionless_text(&format!(
        "rcon {} {}",
        source_command_text(password)?,
        source_command_text(command)?
    ))
}

/// Outgoing datagram (`Q3OutgoingDatagram`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3OutgoingDatagram {
    /// Destination.
    pub to: NetworkAddress,
    /// Payload.
    pub payload: Vec<u8>,
}

/// Address text for userinfo (`addressText`; IPv6 is a local extension).
#[must_use]
pub fn q3_address_text(address: &NetworkAddress) -> String {
    match address {
        NetworkAddress::Loopback { .. } => "loopback".to_string(),
        NetworkAddress::Ipv4 { host, port } => {
            format!("{}.{}.{}.{}:{port}", host[0], host[1], host[2], host[3])
        }
        NetworkAddress::Ipx { network, node, port } => format!(
            "{network:08x}.{}:{port}",
            node.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        ),
        NetworkAddress::Ipv6 { host, port } => format!("[{host}]:{port}"),
    }
}

/// Whether an address is LAN (`q3IsLanAddress`).
#[must_use]
pub fn q3_is_lan_address(address: &NetworkAddress) -> bool {
    match address {
        NetworkAddress::Ipv4 { host, .. } => {
            host[0] == 127
                || host[0] == 10
                || (host[0] == 192 && host[1] == 168)
                || (host[0] == 172 && (16..=31).contains(&host[1]))
        }
        _ => true,
    }
}

/// Read an info value (`q3InfoValue`).
pub fn q3_info_value(info: &str, key: &str) -> Result<String, Q3NetError> {
    if info.len() >= 8192 {
        return Err(Q3NetError::drop("drop", "Info_ValueForKey: oversize infostring"));
    }
    let sanitized = source_command_text(info)?;
    let fields: Vec<&str> = sanitized.split('\\').collect();
    let mut index = if info.starts_with('\\') { 1 } else { 0 };
    while index + 1 < fields.len() {
        if fields[index].to_lowercase() == key.to_lowercase() {
            return Ok(fields[index + 1].to_string());
        }
        index += 2;
    }
    Ok(String::new())
}

fn info_set(
    info: &str,
    key: &str,
    value: &str,
    print: &mut dyn FnMut(&str),
) -> Result<String, Q3NetError> {
    Ok(set_info_value(
        info,
        key,
        value,
        InfoOptions {
            dialect: Dialect::Q3,
            maximum_length: 1024,
            target: InfoTarget::ClientUserinfo,
            server_high_characters: false,
        },
        print,
    )?)
}

/// Client admission result (`Q3ClientAdmissionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3ClientAdmissionResult {
    /// Handled internally.
    Handled,
    /// Admitted.
    Admitted {
        /// Address.
        address: NetworkAddress,
        /// Challenge.
        challenge: i32,
        /// Qport.
        qport: u16,
    },
    /// Other connectionless packet.
    Connectionless {
        /// Packet.
        packet: ConnectionlessPacket,
    },
    /// Sequenced bytes for the connection.
    Sequenced {
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// Ignored.
    Ignored,
}

/// Client admission phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ClientAdmissionPhase {
    /// Disconnected.
    Disconnected,
    /// Connecting.
    Connecting,
    /// Challenging.
    Challenging,
    /// Connected.
    Connected,
}

/// Client admission (`Q3ClientAdmission`).
pub struct Q3ClientAdmission<'a> {
    /// Qport.
    pub qport: u16,
    print: Box<dyn FnMut(&str) + 'a>,
    /// Phase.
    pub phase: Q3ClientAdmissionPhase,
    /// Address.
    pub address: Option<NetworkAddress>,
    /// Challenge.
    pub challenge: i32,
    /// Connect time.
    pub connect_time: i32,
    /// Connect packet count.
    pub connect_packet_count: i32,
    /// Last packet time.
    pub last_packet_time: i32,
}

impl<'a> Q3ClientAdmission<'a> {
    /// Build admission.
    pub fn new(qport: u16, print: impl FnMut(&str) + 'a) -> Self {
        Self {
            qport,
            print: Box::new(print),
            phase: Q3ClientAdmissionPhase::Disconnected,
            address: None,
            challenge: 0,
            connect_time: -99999,
            connect_packet_count: 0,
            last_packet_time: 0,
        }
    }

    /// Begin connecting.
    pub fn begin(&mut self, address: NetworkAddress) {
        self.phase = if matches!(address, NetworkAddress::Loopback { .. }) {
            Q3ClientAdmissionPhase::Challenging
        } else {
            Q3ClientAdmissionPhase::Connecting
        };
        self.address = Some(address);
        self.connect_time = -99999;
        self.connect_packet_count = 0;
    }

    /// Resend a challenge/connect request when due.
    pub fn resend(&mut self, now: i32, userinfo: &str) -> Result<Option<Q3OutgoingDatagram>, Q3NetError> {
        if (self.phase != Q3ClientAdmissionPhase::Connecting
            && self.phase != Q3ClientAdmissionPhase::Challenging)
            || now.wrapping_sub(self.connect_time) < 3000
        {
            return Ok(None);
        }
        let Some(address) = self.address.clone() else {
            return Err(Q3NetError::Protocol("Connection resend has no resolved address"));
        };
        self.connect_time = now;
        self.connect_packet_count = self.connect_packet_count.wrapping_add(1);
        if self.phase == Q3ClientAdmissionPhase::Connecting {
            return Ok(Some(Q3OutgoingDatagram {
                to: address,
                payload: encode_connectionless_text("getchallenge")?,
            }));
        }
        let mut info: String = source_command_text(userinfo)?.chars().take(1023).collect();
        info = info_set(&info, "protocol", "68", &mut self.print)?;
        info = info_set(&info, "qport", &self.qport.to_string(), &mut self.print)?;
        let challenge = self.challenge;
        info = info_set(&info, "challenge", &challenge.to_string(), &mut self.print)?;
        Ok(Some(Q3OutgoingDatagram { to: address, payload: encode_connect(&info)? }))
    }

    /// Receive a datagram.
    pub fn receive(
        &mut self,
        from: NetworkAddress,
        bytes: &[u8],
        now: i32,
    ) -> Result<Q3ClientAdmissionResult, Q3NetError> {
        self.last_packet_time = now;
        if bytes.len() >= 4 && bytes[..4] == [255, 255, 255, 255] {
            let packet = decode_connectionless(bytes, ConnectionlessReceiver::Client)?;
            match packet.command.to_lowercase().as_str() {
                "challengeresponse" => {
                    if self.phase == Q3ClientAdmissionPhase::Connecting {
                        self.challenge =
                            native_atoi(packet.arguments.first().map(String::as_str).unwrap_or(""))?;
                        self.phase = Q3ClientAdmissionPhase::Challenging;
                        self.connect_packet_count = 0;
                        self.connect_time = -99999;
                        self.address = Some(from);
                    }
                    return Ok(Q3ClientAdmissionResult::Handled);
                }
                "connectresponse" => {
                    if self.phase != Q3ClientAdmissionPhase::Challenging
                        || self.address.as_ref().is_none_or(|address| !same_address(&from, address, false))
                    {
                        return Ok(Q3ClientAdmissionResult::Ignored);
                    }
                    self.phase = Q3ClientAdmissionPhase::Connected;
                    self.address = Some(from.clone());
                    return Ok(Q3ClientAdmissionResult::Admitted {
                        address: from,
                        challenge: self.challenge,
                        qport: self.qport,
                    });
                }
                _ => return Ok(Q3ClientAdmissionResult::Connectionless { packet }),
            }
        }
        if self.phase != Q3ClientAdmissionPhase::Connected
            || self.address.as_ref().is_none_or(|address| !same_address(&from, address, true))
        {
            return Ok(Q3ClientAdmissionResult::Ignored);
        }
        Ok(Q3ClientAdmissionResult::Sequenced { bytes: bytes.to_vec() })
    }

    /// Disconnect.
    pub fn disconnect(&mut self) {
        self.phase = Q3ClientAdmissionPhase::Disconnected;
        self.address = None;
    }
}

/// Challenge slot (`Q3Challenge`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3Challenge {
    /// Address.
    pub address: Option<NetworkAddress>,
    /// Challenge.
    pub challenge: i32,
    /// Time.
    pub time: i32,
    /// First time.
    pub first_time: i32,
    /// Ping time.
    pub ping_time: i32,
    /// Connected.
    pub connected: bool,
}

/// Admission slot view (`Q3AdmissionSlot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3AdmissionSlot {
    /// Slot index.
    pub slot: i32,
    /// Phase.
    pub phase: Q3SlotPhase,
    /// Address.
    pub address: Option<NetworkAddress>,
    /// Bot.
    pub bot: bool,
    /// Qport.
    pub qport: u16,
    /// Last connect time.
    pub last_connect_time: i32,
}

/// Slot phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SlotPhase {
    /// Free.
    Free,
    /// Zombie.
    Zombie,
    /// Connected.
    Connected,
    /// Primed.
    Primed,
    /// Active.
    Active,
}

/// Accepted connect (`Q3AcceptedConnect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3AcceptedConnect {
    /// Slot.
    pub slot: i32,
    /// Address.
    pub address: NetworkAddress,
    /// Challenge.
    pub challenge: i32,
    /// Qport.
    pub qport: u16,
    /// Userinfo.
    pub userinfo: String,
}

/// Server admission bindings (`Q3ServerAdmissionBindings`).
pub trait Q3ServerAdmissionBindings {
    /// Whether the server runs.
    fn enabled(&self) -> bool;
    /// Slot views.
    fn slots(&self) -> Vec<Q3AdmissionSlot>;
    /// Private clients.
    fn private_clients(&self) -> i32;
    /// Private password.
    fn private_password(&self) -> String;
    /// Reconnect limit seconds.
    fn reconnect_limit_seconds(&self) -> i32;
    /// Minimum ping.
    fn minimum_ping(&self) -> f32;
    /// Maximum ping.
    fn maximum_ping(&self) -> f32;
    /// Authorize address.
    fn authorize_address(&self) -> Option<NetworkAddress>;
    /// Demo restricted.
    fn demo_restricted(&self) -> bool;
    /// Whether an address is LAN.
    fn is_lan(&self, address: &NetworkAddress) -> bool;
    /// Random int32.
    fn random(&mut self) -> i32;
    /// Authorize a challenge.
    fn authorize(&mut self, challenge: &Q3Challenge);
    /// Send a packet.
    fn send(&mut self, address: &NetworkAddress, packet: &[u8]);
    /// Admit a connection; returns rejection text or `None`.
    fn admit(&mut self, connection: &Q3AcceptedConnect) -> Option<String>;
    /// Drop a bot.
    fn drop_bot(&mut self, slot: i32);
    /// Print.
    fn print(&mut self, text: &str);
    /// Handle other connectionless packets.
    fn query(&mut self, from: &NetworkAddress, packet: &ConnectionlessPacket);
}

/// Server admission (`Q3ServerAdmission`).
pub struct Q3ServerAdmission<'a> {
    /// Challenge storage.
    pub challenges: Vec<Q3Challenge>,
    bindings: &'a mut dyn Q3ServerAdmissionBindings,
}

impl<'a> Q3ServerAdmission<'a> {
    /// Build admission over bindings.
    pub fn new(bindings: &'a mut dyn Q3ServerAdmissionBindings) -> Self {
        Self {
            challenges: vec![
                Q3Challenge {
                    address: None,
                    challenge: 0,
                    time: 0,
                    first_time: 0,
                    ping_time: 0,
                    connected: false,
                };
                1024
            ],
            bindings,
        }
    }

    fn reply(&mut self, to: &NetworkAddress, text: &str) -> Result<(), Q3NetError> {
        let packet = encode_connectionless_text(text)?;
        self.bindings.send(to, &packet);
        Ok(())
    }

    fn challenge(&mut self, from: &NetworkAddress, now: i32) -> Result<(), Q3NetError> {
        if !self.bindings.enabled() {
            return Ok(());
        }
        let mut oldest_time = i32::MAX;
        let mut oldest = 0;
        let mut found = None;
        for (index, candidate) in self.challenges.iter().enumerate() {
            if !candidate.connected
                && candidate.address.as_ref().is_some_and(|address| same_address(from, address, true))
            {
                found = Some(index);
                break;
            }
            if candidate.time < oldest_time {
                oldest_time = candidate.time;
                oldest = index;
            }
        }
        let index = found.unwrap_or(oldest);
        if found.is_none() {
            let random_high = self.bindings.random();
            let random_low = self.bindings.random();
            let slot = &mut self.challenges[index];
            slot.challenge = (random_high << 16) ^ random_low ^ now;
            slot.address = Some(from.clone());
            slot.first_time = now;
            slot.time = now;
            slot.connected = false;
        }
        if self.bindings.is_lan(from) || now.wrapping_sub(self.challenges[index].first_time) > 5000 {
            self.challenges[index].ping_time = now;
            let challenge = self.challenges[index].challenge;
            self.reply(from, &format!("challengeResponse {challenge}"))?;
            return Ok(());
        }
        let snapshot = self.challenges[index].clone();
        self.bindings.authorize(&snapshot);
        Ok(())
    }

    fn authorize(
        &mut self,
        from: &NetworkAddress,
        packet: &ConnectionlessPacket,
        now: i32,
    ) -> Result<(), Q3NetError> {
        let authority = self.bindings.authorize_address();
        if !matches!(from, NetworkAddress::Ipv4 { .. })
            || authority.as_ref().is_none_or(|authority| {
                !matches!(authority, NetworkAddress::Ipv4 { .. })
                    || !same_address(from, authority, false)
            })
        {
            return Ok(());
        }
        let number = native_atoi(packet.arguments.first().map(String::as_str).unwrap_or(""))?;
        let Some(index) = self.challenges.iter().position(|value| value.challenge == number) else {
            return Ok(());
        };
        if self.challenges[index].address.is_none() {
            return Ok(());
        }
        self.challenges[index].ping_time = now;
        let result = packet.arguments.get(1).cloned().unwrap_or_default().to_lowercase();
        let reason = packet.arguments.get(2).cloned().unwrap_or_default();
        if result == "accept" || (result == "demo" && self.bindings.demo_restricted()) {
            let address = self.challenges[index].address.clone().expect("checked");
            let challenge = self.challenges[index].challenge;
            self.reply(&address, &format!("challengeResponse {challenge}"))?;
        } else {
            let address = self.challenges[index].address.clone().expect("checked");
            if result == "demo" {
                self.reply(&address, "print\nServer is not a demo server\n")?;
            } else {
                self.reply(&address, &format!("print\n{reason}\n"))?;
            }
            self.challenges[index] = Q3Challenge {
                address: None,
                challenge: 0,
                time: 0,
                first_time: 0,
                ping_time: 0,
                connected: false,
            };
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn connect(&mut self, from: &NetworkAddress, input: &str, now: i32) -> Result<(), Q3NetError> {
        let mut userinfo: String =
            source_command_text(&source_command_text(input)?)?.chars().take(1023).collect();
        if native_atoi(&q3_info_value(&userinfo, "protocol")?)? != 68 {
            self.reply(from, "print\nServer uses protocol version 68.\n")?;
            return Ok(());
        }
        let qport = native_atoi(&q3_info_value(&userinfo, "qport")?)?;
        let challenge_number = native_atoi(&q3_info_value(&userinfo, "challenge")?)?;
        if qport < 0 || qport > 65535 {
            self.reply(from, "print\nInvalid qport.\n")?;
            return Ok(());
        }
        let matches = |slot: &Q3AdmissionSlot| {
            slot.address.as_ref().is_some_and(|address| {
                same_address(from, address, false)
                    && (i32::from(slot.qport) == qport
                        || (!matches!(from, NetworkAddress::Loopback { .. })
                            && !matches!(address, NetworkAddress::Loopback { .. })
                            && q3_ports_equal(from, address)))
            })
        };
        let slots = self.bindings.slots();
        let existing = slots.iter().find(|slot| slot.phase != Q3SlotPhase::Free && matches(slot));
        if existing.is_some_and(|existing| {
            now.wrapping_sub(existing.last_connect_time)
                < self.bindings.reconnect_limit_seconds().wrapping_mul(1000)
        }) {
            return Ok(());
        }
        if !matches!(from, NetworkAddress::Loopback { .. }) {
            let found = self.challenges.iter().position(|value| {
                value.address.as_ref().is_some_and(|address| same_address(from, address, true))
                    && value.challenge == challenge_number
            });
            let Some(index) = found else {
                self.reply(from, "print\nNo or bad challenge for address.\n")?;
                return Ok(());
            };
            let address_text = q3_address_text(from);
            userinfo = info_set(&userinfo, "ip", &address_text, &mut |text| {
                self.bindings.print(text);
            })?;
            let ping = now.wrapping_sub(self.challenges[index].ping_time);
            self.challenges[index].connected = true;
            if !self.bindings.is_lan(from) {
                if self.bindings.minimum_ping() != 0.0 && (ping as f32) < self.bindings.minimum_ping() {
                    self.reply(from, "print\nServer is for high pings only\n")?;
                    self.challenges[index].address = Some(zero_port(from));
                    return Ok(());
                }
                if self.bindings.maximum_ping() != 0.0 && (ping as f32) > self.bindings.maximum_ping() {
                    self.reply(from, "print\nServer is for low pings only\n")?;
                    return Ok(());
                }
            }
        } else {
            userinfo = info_set(&userinfo, "ip", "localhost", &mut |text| {
                self.bindings.print(text);
            })?;
        }
        let password_ok =
            q3_info_value(&userinfo, "password")? == self.bindings.private_password();
        let start = if password_ok { 0 } else { self.bindings.private_clients() };
        let mut selected = existing.cloned().or_else(|| {
            slots.iter().find(|slot| slot.slot >= start && slot.phase == Q3SlotPhase::Free).cloned()
        });
        if selected.is_none() {
            if !matches!(from, NetworkAddress::Loopback { .. }) {
                self.reply(from, "print\nServer is full.\n")?;
                return Ok(());
            }
            let candidates: Vec<&Q3AdmissionSlot> =
                slots.iter().filter(|slot| slot.slot >= start).collect();
            if candidates.iter().any(|slot| !slot.bot) {
                return Err(Q3NetError::drop("fatal", "server is full on local connect\n"));
            }
            selected = candidates.last().cloned().cloned();
            let Some(selected_slot) = selected.as_ref() else {
                return Err(Q3NetError::drop("fatal", "server is full on local connect\n"));
            };
            let slot = selected_slot.slot;
            self.bindings.drop_bot(slot);
            if !self.bindings.enabled() {
                return Ok(());
            }
        }
        let selected = selected.expect("selected");
        let accepted = Q3AcceptedConnect {
            slot: selected.slot,
            address: from.clone(),
            qport: qport as u16,
            challenge: challenge_number,
            userinfo,
        };
        let rejected = self.bindings.admit(&accepted);
        if !self.bindings.enabled() {
            return Ok(());
        }
        if let Some(rejected) = rejected {
            self.reply(from, &format!("print\n{rejected}\n"))?;
            return Ok(());
        }
        self.reply(from, "connectResponse")?;
        Ok(())
    }

    /// Receive a connectionless datagram.
    pub fn receive(
        &mut self,
        from: &NetworkAddress,
        bytes: &[u8],
        now: i32,
    ) -> Result<(), Q3NetError> {
        let packet = decode_connectionless(bytes, ConnectionlessReceiver::Server)?;
        match packet.command.to_lowercase().as_str() {
            "getchallenge" => self.challenge(from, now),
            "ipauthorize" => self.authorize(from, &packet, now),
            "connect" => {
                let input = packet.arguments.first().cloned().unwrap_or_default();
                self.connect(from, &input, now)
            }
            _ => {
                self.bindings.query(from, &packet);
                Ok(())
            }
        }
    }

    /// Mark a challenge disconnected.
    pub fn disconnect(&mut self, address: &NetworkAddress) {
        if let Some(challenge) = self.challenges.iter_mut().find(|value| {
            value.address.as_ref().is_some_and(|current| same_address(address, current, true))
        }) {
            challenge.connected = false;
        }
    }
}

/// Clone an address with its port zeroed.
fn zero_port(address: &NetworkAddress) -> NetworkAddress {
    match address {
        NetworkAddress::Ipv4 { host, .. } => NetworkAddress::Ipv4 { host: *host, port: 0 },
        NetworkAddress::Ipv6 { host, .. } => {
            NetworkAddress::Ipv6 { host: host.clone(), port: 0 }
        }
        NetworkAddress::Ipx { network, node, .. } => {
            NetworkAddress::Ipx { network: *network, node: *node, port: 0 }
        }
        NetworkAddress::Loopback { id } => NetworkAddress::Loopback { id: id.clone() },
    }
}

fn q3_ports_equal(left: &NetworkAddress, right: &NetworkAddress) -> bool {
    match (left, right) {
        (
            NetworkAddress::Ipv4 { port: left, .. },
            NetworkAddress::Ipv4 { port: right, .. },
        )
        | (
            NetworkAddress::Ipx { port: left, .. },
            NetworkAddress::Ipx { port: right, .. },
        )
        | (
            NetworkAddress::Ipv6 { port: left, .. },
            NetworkAddress::Ipv6 { port: right, .. },
        ) => left == right,
        (
            NetworkAddress::Loopback { id: left },
            NetworkAddress::Loopback { id: right },
        ) => left == right,
        _ => false,
    }
}

/// Packet routing uses qport before accepting NAT port rebinding (`routeQ3SequencedPacket`).
#[must_use]
pub fn route_q3_sequenced_packet(
    from: &NetworkAddress,
    bytes: &[u8],
    slots: &[Q3AdmissionSlot],
) -> Option<Q3AdmissionSlot> {
    if bytes.len() < 6 {
        return None;
    }
    let qport = u16::from_le_bytes([bytes[4], bytes[5]]);
    slots
        .iter()
        .find(|slot| {
            slot.phase != Q3SlotPhase::Free
                && slot.address.as_ref().is_some_and(|address| same_address(from, address, false))
                && slot.qport == qport
        })
        .cloned()
}

/// Connection identity (`Q3ConnectionIdentity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q3ConnectionIdentity {
    /// Client.
    pub client: ClientId,
    /// Seat.
    pub seat: Option<SeatId>,
}

/// Client bindings (`Q3ClientBindings`).
pub trait Q3ClientBindings {
    /// Assert the session is current.
    fn assert_current(&mut self);
    /// Print.
    fn print(&mut self, text: &str);
    /// Clear active state.
    fn clear_active(&mut self);
    /// Apply system info.
    fn system_info(&mut self, info: &str);
    /// Apply a gamestate.
    fn gamestate(&mut self, state: &Gamestate, generation: i32);
    /// Apply a snapshot.
    fn snapshot(&mut self, snapshot: &Snapshot, ping: i32);
    /// Publish a download size.
    fn download_size(&mut self, size: i32) -> i32;
    /// Receive a download block.
    fn download(&mut self, block: &DownloadBlock);
    /// Restart the map.
    fn map_restart(&mut self);
    /// Take a level shot.
    fn level_shot(&mut self);
    /// Whether a local server runs.
    fn local_server_running(&self) -> bool;
}

/// Client packet result (`Q3ClientPacketResult`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ClientPacketResult {
    /// Complete message.
    Accepted {
        /// Sequence.
        sequence: i32,
        /// Dropped packets.
        dropped: i32,
        /// Message.
        message: ServerMessage,
        /// Plaintext.
        plaintext: Vec<u8>,
    },
    /// Fragment buffered.
    Fragment {
        /// Sequence.
        sequence: i32,
        /// Bytes received.
        received: usize,
    },
    /// Rejected.
    Rejected {
        /// Reason.
        reason: ChannelReject,
    },
}

/// Client mode (`Q3ClientMode`).
pub enum Q3ClientMode<'a> {
    /// Network.
    Network {
        /// Challenge.
        challenge: i32,
        /// Qport.
        qport: u16,
    },
    /// Demo.
    Demo {
        /// Reader.
        reader: Box<dyn DemoMessageReader + 'a>,
    },
}

/// Command history (`Q3CommandHistory`).
#[derive(Debug, Clone)]
pub struct Q3CommandHistory {
    commands: Vec<WireUserCommand>,
    number: i32,
}

impl Q3CommandHistory {
    /// Fresh history.
    #[must_use]
    pub fn new() -> Self {
        Self { commands: vec![WireUserCommand::default(); 64], number: 0 }
    }

    /// Current number.
    #[must_use]
    pub fn current_number(&self) -> i32 {
        self.number
    }

    /// Append a command.
    pub fn append(&mut self, command: &WireUserCommand) -> i32 {
        self.number = self.number.wrapping_add(1);
        self.commands[(self.number & 63) as usize] = command.clone();
        self.number
    }

    /// Read a command.
    pub fn read(&self, number: i32) -> Result<Option<WireUserCommand>, Q3NetError> {
        if number > self.number {
            return Err(Q3NetError::drop(
                "drop",
                "CL_GetUserCmd: requested future command",
            ));
        }
        if number <= self.number.wrapping_sub(64) {
            return Ok(None);
        }
        self.commands
            .get((number & 63) as usize)
            .cloned()
            .map(Some)
            .ok_or(Q3NetError::Range("Missing command ring slot"))
    }

    /// Restart commands without resetting the number.
    pub fn restart(&mut self) {
        self.commands.fill(WireUserCommand::default());
    }

    /// Clear commands and number.
    pub fn clear(&mut self) {
        self.restart();
        self.number = 0;
    }
}

impl Default for Q3CommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
struct SentPacket {
    command_number: i32,
    server_time: i32,
    real_time: i32,
}

/// Client send options (`Q3ClientSendOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ClientSendOptions {
    /// Real time.
    pub real_time: i32,
    /// Packet duplication.
    pub packet_dup: i32,
    /// No delta.
    pub no_delta: bool,
}

/// Client send readiness (`Q3ClientSendReadiness`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ClientSendReadiness {
    /// Real time.
    pub real_time: i32,
    /// Active.
    pub active: bool,
    /// Primed.
    pub primed: bool,
    /// Cinematic.
    pub cinematic: bool,
    /// Downloading.
    pub downloading: bool,
    /// Local.
    pub local: bool,
    /// LAN.
    pub lan: bool,
    /// Maximum packets.
    pub maximum_packets: i32,
}

/// Client connection (`Q3ClientConnection`).
pub struct Q3ClientConnection<'a> {
    /// Identity.
    pub identity: Q3ConnectionIdentity,
    /// Product.
    pub product: Q3Product,
    /// Reliable commands.
    pub reliable: ClientReliableCommands,
    /// Parse entities.
    pub parse_entities: SourceParseEntities,
    /// History.
    pub history: SnapshotHistory<'a>,
    /// Game state.
    pub game_state: ClientGameStateStorage,
    /// Commands.
    pub commands: Q3CommandHistory,
    /// Source state.
    pub source_state: SourceMessageState,
    /// Baselines.
    pub baselines: Vec<Q3EntityState>,
    /// Channel (network mode).
    pub channel: Option<Netchannel>,
    server_commands: Vec<String>,
    out_packets: Vec<SentPacket>,
    snapshot_pings: HashMap<i32, i32>,
    server_command_executor: Q3ServerCommandExecutor,
    /// Server message sequence.
    pub server_message_sequence: i32,
    /// Server command sequence.
    pub server_command_sequence: i32,
    /// Last executed server command.
    pub last_executed_server_command: i32,
    /// Server id.
    pub server_id: i32,
    /// Checksum feed.
    pub checksum_feed: i32,
    /// Client number.
    pub client_number: i32,
    /// Generation.
    pub generation: i32,
    /// Demo waiting.
    pub demo_waiting: bool,
    /// Last packet sent time.
    pub last_packet_sent_time: i32,
    mode: Q3ClientMode<'a>,
    bindings: &'a mut dyn Q3ClientBindings,
}

/// Snapshot ping over split borrows (`ping`).
fn snapshot_ping_for(
    channel: &Option<Netchannel>,
    out_packets: &[SentPacket],
    snapshot: &Snapshot,
    real_time: i32,
) -> i32 {
    for index in 0..32 {
        let outgoing = channel.as_ref().map_or(1, |channel| channel.outgoing_sequence());
        let slot = outgoing.wrapping_sub(1).wrapping_sub(index) & 31;
        let Some(packet) = out_packets.get(slot as usize) else {
            continue;
        };
        if snapshot.player_state.command_time >= packet.server_time {
            return real_time.wrapping_sub(packet.real_time);
        }
    }
    999
}

/// Apply system info over split borrows (`applySystemInfo`).
fn apply_q3_system_info(
    game_state: &ClientGameStateStorage,
    server_id: &mut i32,
    bindings: &mut dyn Q3ClientBindings,
) -> Result<(), Q3NetError> {
    let info = game_state.get(1)?.unwrap_or_default();
    let fields: Vec<&str> = info.split('\\').collect();
    *server_id = 0;
    let mut index = if info.starts_with('\\') { 1 } else { 0 };
    while index + 1 < fields.len() {
        if fields[index].to_lowercase() == "sv_serverid" {
            *server_id = native_atoi(fields[index + 1])?;
            break;
        }
        index += 2;
    }
    bindings.system_info(&info);
    Ok(())
}

impl<'a> Q3ClientConnection<'a> {
    /// Build a connection.
    pub fn new(
        identity: Q3ConnectionIdentity,
        product: Q3Product,
        mode: Q3ClientMode<'a>,
        bindings: &'a mut dyn Q3ClientBindings,
    ) -> Self {
        let channel = match &mode {
            Q3ClientMode::Network { qport, .. } => {
                let qport = *qport;
                Some(Netchannel::new(ChannelRole::Client, qport, move || qport))
            }
            Q3ClientMode::Demo { .. } => None,
        };
        Self {
            identity,
            product,
            reliable: ClientReliableCommands::new(),
            parse_entities: SourceParseEntities::new(),
            history: SnapshotHistory::new(None),
            game_state: ClientGameStateStorage::new(),
            commands: Q3CommandHistory::new(),
            source_state: SourceMessageState::new(|_| {}),
            baselines: vec![Q3EntityState::default(); 1024],
            channel,
            server_commands: vec![String::new(); 64],
            out_packets: vec![
                SentPacket { command_number: 0, server_time: 0, real_time: 0 };
                32
            ],
            snapshot_pings: HashMap::new(),
            server_command_executor: Q3ServerCommandExecutor::new(),
            server_message_sequence: 0,
            server_command_sequence: 0,
            last_executed_server_command: 0,
            server_id: 0,
            checksum_feed: 0,
            client_number: 0,
            generation: 0,
            demo_waiting: true,
            last_packet_sent_time: 0,
            mode,
            bindings,
        }
    }

    /// Mode discriminant.
    #[must_use]
    pub fn is_demo(&self) -> bool {
        matches!(self.mode, Q3ClientMode::Demo { .. })
    }

    fn server_command(&self, sequence: i32) -> Result<String, Q3NetError> {
        self.server_commands
            .get((sequence & 63) as usize)
            .cloned()
            .ok_or(Q3NetError::Range("Missing server command ring slot"))
    }

    /// Snapshot ping.
    pub fn snapshot_ping(&self, number: i32) -> Result<Option<i32>, Q3NetError> {
        let entry = self.history.read_slot(number)?;
        Ok(match entry {
            Some(entry)
                if entry.status == SnapshotStatus::Valid
                    && entry.snapshot.message_number == number =>
            {
                self.snapshot_pings.get(&number).copied()
            }
            _ => None,
        })
    }

    /// Copy the gamestate.
    pub fn copy_gamestate(&self) -> Result<Gamestate, Q3NetError> {
        let mut entries = Vec::new();
        for index in 0..1024 {
            if let Some(value) = self.game_state.get(index)? {
                entries.push(GamestateEntry::Configstring { index: index as i32, value });
            }
        }
        for (number, entity) in self.baselines.iter().enumerate() {
            if entity.number != 0 {
                entries.push(GamestateEntry::Baseline { number: number as i32, entity: entity.clone() });
            }
        }
        Ok(Gamestate {
            command_sequence: self.server_command_sequence,
            entries,
            client_number: self.client_number,
            checksum_feed: self.checksum_feed,
        })
    }

    /// Accepts plaintext from channel decode or a demo record.
    pub fn receive_message(
        &mut self,
        sequence: i32,
        bytes: &[u8],
        real_time: i32,
    ) -> Result<ServerMessage, Q3NetError> {
        self.bindings.assert_current();
        if matches!(self.mode, Q3ClientMode::Network { .. })
            && (sequence <= self.server_message_sequence || sequence < 1)
        {
            return Err(Q3NetError::Range("Server message sequence must advance"));
        }
        self.server_message_sequence = sequence;
        let read_offset = if matches!(self.mode, Q3ClientMode::Network { .. }) { 4 } else { 0 };
        // The cursor borrows the parse-entity ring while steps borrow the
        // baselines/history; split the borrows through a helper scope.
        let result = self.receive_message_inner(bytes, read_offset, real_time)?;
        Ok(result)
    }

    fn receive_message_inner(
        &mut self,
        bytes: &[u8],
        read_offset: usize,
        real_time: i32,
    ) -> Result<ServerMessage, Q3NetError> {
        // NLL: the cursor holds the parse-entity ring while steps borrow the
        // baselines/history; split the borrows through a helper scope. The
        // download-size publish runs manually in the download arm so the
        // cursor never borrows the bindings.
        let Q3ClientConnectionFields {
            bindings,
            history,
            parse_entities,
            game_state,
            commands,
            baselines,
            reliable,
            snapshot_pings,
            product,
            channel,
            server_commands,
            out_packets,
            server_message_sequence,
            server_command_sequence,
            client_number,
            checksum_feed,
            server_id,
            generation,
            demo_waiting,
        } = self.fields();
        let mut cursor =
            ServerMessageCursor::with_diagnostics(bytes, Some(parse_entities), None, None, read_offset)?;
        let mut acknowledge = 0;
        let mut command_sequence = 0;
        let mut client_number_value = 0;
        let mut checksum_feed_value = 0;
        let mut entries = Vec::new();
        let mut operations = Vec::new();
        loop {
            bindings.assert_current();
            let baseline_lookup;
            let history_lookup;
            let step = {
                baseline_lookup = |number: i32| {
                    (number >= 0).then(|| baselines.get(number as usize).cloned()).flatten()
                };
                history_lookup =
                    |number: i32| history.read_or_zero(number, *product).ok();
                cursor.next(&ServerMessageContext {
                    product: *product,
                    message_number: *server_message_sequence,
                    reliable_sequence: reliable.sequence(),
                    server_command_sequence: *server_command_sequence,
                    parse_entities_number: cursor.parse_entities_number(),
                    baseline: &baseline_lookup,
                    history: &history_lookup,
                })?
            };
            match step {
                ServerMessageStep::Acknowledge(sequence) => {
                    acknowledge = sequence;
                    reliable.assign_acknowledgement(sequence);
                }
                ServerMessageStep::GamestateStart => {
                    bindings.clear_active();
                    bindings.assert_current();
                    history.clear();
                    snapshot_pings.clear();
                    cursor.clear_parse_entities();
                    game_state.clear();
                    commands.clear();
                    *server_id = 0;
                    baselines.fill(Q3EntityState::default());
                    out_packets.fill(SentPacket {
                        command_number: 0,
                        server_time: 0,
                        real_time: 0,
                    });
                    *generation = generation.wrapping_add(1);
                    entries = Vec::new();
                }
                ServerMessageStep::GamestateSequence(sequence) => {
                    command_sequence = sequence;
                    *server_command_sequence = sequence;
                    game_state.begin_entries();
                }
                ServerMessageStep::GamestateEntry(entry) => {
                    entries.push(entry.clone());
                    match &entry {
                        GamestateEntry::Configstring { index, value } => {
                            game_state.append(*index as usize, value)?;
                        }
                        GamestateEntry::Baseline { number, entity } => {
                            let Some(baseline) = (*number >= 0)
                                .then(|| baselines.get_mut(*number as usize))
                                .flatten()
                            else {
                                return Err(Q3NetError::Range("Invalid source baseline number"));
                            };
                            *baseline = entity.clone();
                        }
                    }
                }
                ServerMessageStep::GamestateClient(number) => {
                    client_number_value = number;
                    *client_number = number;
                }
                ServerMessageStep::GamestateChecksum(checksum) => {
                    checksum_feed_value = checksum;
                    *checksum_feed = checksum;
                }
                ServerMessageStep::GamestateEnd => {
                    let state = Gamestate {
                        command_sequence,
                        entries: std::mem::take(&mut entries),
                        client_number: client_number_value,
                        checksum_feed: checksum_feed_value,
                    };
                    operations.push(ServerOperation::Gamestate(state.clone()));
                    apply_q3_system_info(game_state, server_id, bindings)?;
                    bindings.assert_current();
                    bindings.gamestate(&state, *generation);
                }
                ServerMessageStep::SnapshotHeader(delta) => {
                    if delta <= 0 {
                        *demo_waiting = false;
                    }
                }
                ServerMessageStep::Operation(operation) => {
                    operations.push(operation.clone());
                    match operation {
                        ServerOperation::Nop => {}
                        ServerOperation::Command { sequence, text } => {
                            *server_command_sequence = sequence;
                            let slot = (sequence & 63) as usize;
                            if let Some(entry) = server_commands.get_mut(slot) {
                                *entry = stored_text(&text);
                            }
                        }
                        ServerOperation::Snapshot { ref snapshot, .. } => {
                            if history.publish(&operation)? {
                                let ping = snapshot_ping_for(
                                    channel,
                                    out_packets,
                                    snapshot,
                                    real_time,
                                );
                                let stale: Vec<i32> = snapshot_pings
                                    .keys()
                                    .copied()
                                    .filter(|number| {
                                        snapshot
                                            .message_number
                                            .wrapping_sub(*number)
                                            >= SNAPSHOT_BACKUP as i32
                                    })
                                    .collect();
                                for number in stale {
                                    snapshot_pings.remove(&number);
                                }
                                snapshot_pings.insert(snapshot.message_number, ping);
                                bindings.snapshot(snapshot, ping);
                            }
                        }
                        ServerOperation::Download(mut block) => {
                            match &mut block {
                                DownloadBlock::Start { file_size, .. }
                                | DownloadBlock::Error { file_size, .. } => {
                                    *file_size = bindings.download_size(*file_size);
                                }
                                DownloadBlock::Chunk { .. } => {}
                            }
                            if let Some(ServerOperation::Download(stored)) = operations.last_mut() {
                                *stored = block.clone();
                            }
                            bindings.download(&block);
                        }
                        ServerOperation::Gamestate(_) => {}
                    }
                }
                ServerMessageStep::End(terminal) => {
                    let parse_entities_number = cursor.parse_entities_number();
                    return Ok(ServerMessage {
                        reliable_acknowledge: acknowledge,
                        server_command_sequence: *server_command_sequence,
                        parse_entities_number,
                        operations,
                        terminal,
                    });
                }
            }
        }
    }

    /// Split field borrows for the message loop.
    #[allow(clippy::type_complexity)]
    fn fields(&mut self) -> Q3ClientConnectionFields<'_, 'a> {
        Q3ClientConnectionFields {
            bindings: &mut *self.bindings,
            history: &mut self.history,
            parse_entities: &mut self.parse_entities,
            game_state: &mut self.game_state,
            commands: &mut self.commands,
            baselines: &mut self.baselines,
            reliable: &mut self.reliable,
            snapshot_pings: &mut self.snapshot_pings,
            product: &self.product,
            channel: &self.channel,
            server_commands: &mut self.server_commands,
            out_packets: &mut self.out_packets,
            server_message_sequence: &mut self.server_message_sequence,
            server_command_sequence: &mut self.server_command_sequence,
            client_number: &mut self.client_number,
            checksum_feed: &mut self.checksum_feed,
            server_id: &mut self.server_id,
            generation: &mut self.generation,
            demo_waiting: &mut self.demo_waiting,
        }
    }

    /// Accept one datagram through the channel (`receiveDatagram`).
    pub fn receive_datagram(
        &mut self,
        packet: &[u8],
        real_time: i32,
    ) -> Result<Q3ClientPacketResult, Q3NetError> {
        self.bindings.assert_current();
        let challenge = match &self.mode {
            Q3ClientMode::Network { challenge, .. } => *challenge,
            Q3ClientMode::Demo { .. } => {
                return Err(Q3NetError::Protocol(
                    "Demo connection cannot receive datagrams",
                ));
            }
        };
        if self.channel.is_none() {
            return Err(Q3NetError::Protocol(
                "Demo connection cannot receive datagrams",
            ));
        }
        let result = self
            .channel
            .as_mut()
            .map(|channel| channel.receive(packet, None))
            .unwrap_or(ChannelResult::Rejected { reason: ChannelReject::Malformed });
        let ChannelResult::Accepted { sequence, dropped, payload, .. } = result else {
            return Ok(match result {
                ChannelResult::Fragment { sequence, received } => {
                    Q3ClientPacketResult::Fragment { sequence, received }
                }
                ChannelResult::Rejected { reason } => {
                    Q3ClientPacketResult::Rejected { reason }
                }
                ChannelResult::Accepted { .. } => {
                    unreachable!("matched above")
                }
            });
        };
        let mut header = Q3MsgReader::new(&payload, MessageMode::Bitstream)?;
        let acknowledge = header.read_long()?;
        let key = self.reliable.lookup_masked(acknowledge);
        let plaintext = xor_server_message(&payload, challenge, sequence, &key);
        let message = self.receive_message(sequence, &plaintext, real_time)?;
        Ok(Q3ClientPacketResult::Accepted { sequence, dropped, message, plaintext })
    }

    /// Read one demo record (`readDemo`).
    pub fn read_demo(&mut self, real_time: i32) -> Result<Q3DemoRead, Q3NetError> {
        let Q3ClientMode::Demo { reader } = &mut self.mode else {
            return Err(Q3NetError::Protocol(
                "Network connection cannot read demo messages",
            ));
        };
        let mut sequence = 0;
        let record = reader.next(&mut |value| sequence = value)?;
        self.server_message_sequence = sequence;
        match record {
            DemoRecord::End(end) => Ok(Q3DemoRead::End(end)),
            DemoRecord::Message(message) => {
                let parsed = self.receive_message(message.sequence, &message.payload, real_time)?;
                Ok(Q3DemoRead::Message(parsed))
            }
        }
    }

    /// Execute one server command at `CG_GetServerCommand` time (`getServerCommand`).
    pub fn get_server_command(
        &mut self,
        sequence: i32,
    ) -> Result<Option<Vec<String>>, Q3NetError> {
        self.bindings.assert_current();
        if sequence <= self.server_command_sequence - 64 {
            if matches!(self.mode, Q3ClientMode::Demo { .. }) {
                return Ok(None);
            }
            return Err(Q3NetError::drop(
                "drop",
                "CL_GetServerCommand: a reliable command was cycled out",
            ));
        }
        if sequence > self.server_command_sequence {
            return Err(Q3NetError::drop(
                "drop",
                "CL_GetServerCommand: requested a command not received",
            ));
        }
        self.last_executed_server_command = sequence;
        let text = self.server_command(sequence)?;
        let mut host = Q3ClientServerCommandHost {
            game_state: &mut self.game_state,
            server_id: &mut self.server_id,
            commands: &mut self.commands,
            bindings: &mut *self.bindings,
        };
        self.server_command_executor.execute(&text, &mut host)
    }

    /// Transmit one packet (`transmit`).
    pub fn transmit(
        &mut self,
        options: Q3ClientSendOptions,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        self.bindings.assert_current();
        let challenge = match &self.mode {
            Q3ClientMode::Network { challenge, .. } => *challenge,
            Q3ClientMode::Demo { .. } => {
                return Err(Q3NetError::Protocol("Demo connection cannot send packets"));
            }
        };
        if self.channel.is_none() {
            return Err(Q3NetError::Protocol("Demo connection cannot send packets"));
        }
        let outgoing = self
            .channel
            .as_ref()
            .map_or(0, |channel| channel.outgoing_sequence());
        let packet_dup = options.packet_dup.clamp(0, 5);
        let old_slot = outgoing.wrapping_sub(1).wrapping_sub(packet_dup) & 31;
        let old_command_number = self
            .out_packets
            .get(old_slot as usize)
            .map(|old| old.command_number)
            .ok_or(Q3NetError::Range("Missing outgoing packet slot"))?;
        let count = 32.min(self.commands.current_number() - old_command_number);
        let mut wire = Vec::new();
        for index in 0..count {
            let number = self.commands.current_number() - count + index + 1;
            let command = self
                .commands
                .read(number)?
                .ok_or(Q3NetError::Protocol("Outgoing user command was overwritten"))?;
            wire.push(command);
        }
        let header = ClientHeader {
            server_id: self.server_id,
            message_acknowledge: self.server_message_sequence,
            reliable_acknowledge: self.server_command_sequence,
        };
        let mut writer = begin_client_message(header.clone(), &self.reliable.pending())?;
        if !wire.is_empty() {
            let latest = self.history.latest();
            let kind = if options.no_delta
                || latest.is_none()
                || self.demo_waiting
                || latest.is_some_and(|snapshot| {
                    snapshot.message_number != self.server_message_sequence
                }) {
                ClientMovementKind::MoveNoDelta
            } else {
                ClientMovementKind::Move
            };
            let movement = ClientMovement { kind, commands: wire.clone() };
            let lookup = |sequence: i32| {
                self.server_commands
                    .get((sequence & 63) as usize)
                    .cloned()
                    .unwrap_or_default()
            };
            let context =
                ClientKeyContext { checksum_feed: self.checksum_feed, server_command: &lookup };
            write_client_movement(&mut writer, &movement, header, &context)?;
            let finished = finish_client_message(&mut writer)?;
            let bytes = xor_client_message(&finished, challenge, &lookup)?;
            let server_time = wire.last().map_or(0, |command| command.server_time);
            let slot = (outgoing & 31) as usize;
            self.out_packets[slot] = SentPacket {
                command_number: self.commands.current_number(),
                server_time,
                real_time: options.real_time,
            };
            self.last_packet_sent_time = options.real_time;
            let Some(channel) = self.channel.as_mut() else {
                return Err(Q3NetError::Protocol("Demo connection cannot send packets"));
            };
            channel.begin_transmit(&bytes, delivery)?;
            while channel.has_unsent_fragments() {
                channel.transmit_next_fragment(delivery)?;
            }
            return Ok(());
        }
        let finished = finish_client_message(&mut writer)?;
        let lookup = |sequence: i32| {
            self.server_commands
                .get((sequence & 63) as usize)
                .cloned()
                .unwrap_or_default()
        };
        let bytes = xor_client_message(&finished, challenge, &lookup)?;
        let slot = (outgoing & 31) as usize;
        self.out_packets[slot] = SentPacket {
            command_number: self.commands.current_number(),
            server_time: 0,
            real_time: options.real_time,
        };
        self.last_packet_sent_time = options.real_time;
        let Some(channel) = self.channel.as_mut() else {
            return Err(Q3NetError::Protocol("Demo connection cannot send packets"));
        };
        channel.begin_transmit(&bytes, delivery)?;
        while channel.has_unsent_fragments() {
            channel.transmit_next_fragment(delivery)?;
        }
        Ok(())
    }

    /// Send readiness (`readyToSend`).
    pub fn ready_to_send(
        &self,
        options: &Q3ClientSendReadiness,
    ) -> Result<bool, Q3NetError> {
        if matches!(self.mode, Q3ClientMode::Demo { .. })
            || self.channel.is_none()
            || options.cinematic
        {
            return Ok(false);
        }
        if options.downloading
            && options.real_time.wrapping_sub(self.last_packet_sent_time) < 50
        {
            return Ok(false);
        }
        if !options.active
            && !options.primed
            && !options.downloading
            && options.real_time.wrapping_sub(self.last_packet_sent_time) < 1000
        {
            return Ok(false);
        }
        if options.local || options.lan {
            return Ok(true);
        }
        let maximum = options.maximum_packets.clamp(15, 125);
        let Some(channel) = self.channel.as_ref() else {
            return Ok(false);
        };
        let slot = channel.outgoing_sequence().wrapping_sub(1) & 31;
        let previous = self
            .out_packets
            .get(slot as usize)
            .ok_or(Q3NetError::Range("Missing outgoing packet slot"))?;
        Ok(options.real_time.wrapping_sub(previous.real_time) >= 1000 / maximum)
    }

    /// Reliable disconnect plus three transmits (`disconnectPackets`).
    pub fn disconnect_packets(
        &mut self,
        options: Q3ClientSendOptions,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        if !matches!(self.mode, Q3ClientMode::Network { .. }) {
            return Ok(());
        }
        self.reliable.add("disconnect")?;
        for _ in 0..3 {
            self.transmit(options, delivery)?;
        }
        Ok(())
    }
}

struct Q3ClientConnectionFields<'b, 'a> {
    bindings: &'b mut dyn Q3ClientBindings,
    history: &'b mut SnapshotHistory<'a>,
    parse_entities: &'b mut SourceParseEntities,
    game_state: &'b mut ClientGameStateStorage,
    commands: &'b mut Q3CommandHistory,
    baselines: &'b mut Vec<Q3EntityState>,
    reliable: &'b mut ClientReliableCommands,
    snapshot_pings: &'b mut HashMap<i32, i32>,
    product: &'b Q3Product,
    channel: &'b Option<Netchannel>,
    server_commands: &'b mut Vec<String>,
    out_packets: &'b mut Vec<SentPacket>,
    server_message_sequence: &'b mut i32,
    server_command_sequence: &'b mut i32,
    client_number: &'b mut i32,
    checksum_feed: &'b mut i32,
    server_id: &'b mut i32,
    generation: &'b mut i32,
    demo_waiting: &'b mut bool,
}

/// Demo read result (`readDemo`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3DemoRead {
    /// Parsed message.
    Message(ServerMessage),
    /// End of demo.
    End(DemoEnd),
}

/// Server-command host over split connection borrows (`getServerCommand`).
struct Q3ClientServerCommandHost<'b> {
    game_state: &'b mut ClientGameStateStorage,
    server_id: &'b mut i32,
    commands: &'b mut Q3CommandHistory,
    bindings: &'b mut dyn Q3ClientBindings,
}

impl Q3ServerCommandBindings for Q3ClientServerCommandHost<'_> {
    fn assert_current(&mut self) {
        self.bindings.assert_current();
    }

    fn system_info(&mut self) -> Result<(), Q3NetError> {
        apply_q3_system_info(self.game_state, self.server_id, self.bindings)
    }

    fn map_restart(&mut self) {
        self.commands.restart();
        self.bindings.map_restart();
    }

    fn local_server_running(&self) -> bool {
        self.bindings.local_server_running()
    }

    fn level_shot(&mut self) {
        self.bindings.level_shot();
    }

    fn game_state(&mut self) -> &mut ClientGameStateStorage {
        self.game_state
    }
}

/// Server bindings (`Q3ServerBindings`).
pub trait Q3ServerBindings {
    /// Assert the session is current.
    fn assert_current(&mut self);
    /// Server id.
    fn server_id(&self) -> i32;
    /// Restarted server id.
    fn restarted_server_id(&self) -> i32;
    /// Checksum feed.
    fn checksum_feed(&self) -> i32;
    /// Whether the server is pure.
    fn pure(&self) -> bool;
    /// Whether this is a debug build.
    fn debug_build(&self) -> bool;
    /// Server time.
    fn time(&self) -> i32;
    /// Whether a local client runs.
    fn client_running(&self) -> bool;
    /// Whether flood protection applies.
    fn flood_protect(&self) -> bool;
    /// Current download name.
    fn download_name(&self) -> String;
    /// Run one reliable client command; `false` ends the message.
    fn command(&mut self, command: &ReliableCommand, client_ok: bool) -> Result<bool, Q3NetError>;
    /// Enter the world with the first user command.
    fn enter_world(&mut self, command: &WireUserCommand) -> Result<(), Q3NetError>;
    /// Think one user command.
    fn think(&mut self, command: &WireUserCommand) -> Result<(), Q3NetError>;
    /// Resend the gamestate.
    fn resend_gamestate(&mut self) -> Result<(), Q3NetError>;
    /// Drop the client.
    fn drop_client(&mut self, reason: &str) -> Result<(), Q3NetError>;
    /// Print.
    fn print(&mut self, text: &str);
}

/// Server rate settings (`Q3ServerRate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ServerRate {
    /// Rate.
    pub rate: i32,
    /// Maximum rate.
    pub max_rate: i32,
    /// Snapshot milliseconds.
    pub snapshot_msec: i32,
    /// Local.
    pub local: bool,
    /// Force LAN timing.
    pub force_lan: bool,
    /// LAN.
    pub lan: bool,
}

/// Snapshot rate interval (`q3RateMilliseconds`).
pub fn q3_rate_milliseconds(
    message_size: i32,
    rate: i32,
    max_rate: i32,
) -> Result<i32, Q3NetError> {
    let message_size = message_size.min(1500);
    let rate = if max_rate != 0 { rate.min(max_rate.max(1000)) } else { rate };
    if rate == 0 {
        return Err(Q3NetError::Range("Source snapshot rate division by zero"));
    }
    Ok(message_size.wrapping_add(48).wrapping_mul(1000).wrapping_div(rate))
}

/// Server connection phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ServerPhase {
    /// Connected.
    Connected,
    /// Primed.
    Primed,
    /// Active.
    Active,
    /// Zombie.
    Zombie,
}

/// Server connection: one admitted client (`Q3ServerConnection`).
pub struct Q3ServerConnection<'a> {
    /// Identity.
    pub identity: Q3ConnectionIdentity,
    /// Challenge.
    pub challenge: i32,
    /// Channel.
    pub channel: Netchannel,
    /// Reliable commands.
    pub reliable: ServerReliableCommands,
    /// Source state.
    pub source_state: SourceMessageState,
    /// Snapshot history.
    pub snapshots: &'a mut Q3ServerSnapshotHistory<'a>,
    queued_messages: Vec<Vec<u8>>,
    /// Phase.
    pub phase: Q3ServerPhase,
    /// Last client command.
    pub last_client_command: i32,
    /// Last client command text.
    pub last_client_command_string: String,
    /// Message acknowledge.
    pub message_acknowledge: i32,
    /// Delta message.
    pub delta_message: i32,
    /// Gamestate message number.
    pub gamestate_message_number: i32,
    /// Last user command.
    pub last_user_command: WireUserCommand,
    /// Pure authentic.
    pub pure_authentic: bool,
    /// Got pure command.
    pub got_pure_command: bool,
    /// Next snapshot time.
    pub next_snapshot_time: i32,
    /// Rate delayed.
    pub rate_delayed: bool,
    /// Reliable sent.
    pub reliable_sent: i32,
    /// Next reliable time.
    pub next_reliable_time: i32,
    bindings: &'a mut dyn Q3ServerBindings,
}

impl<'a> Q3ServerConnection<'a> {
    /// Build a connection.
    pub fn new(
        identity: Q3ConnectionIdentity,
        challenge: i32,
        qport: u16,
        snapshots: &'a mut Q3ServerSnapshotHistory<'a>,
        bindings: &'a mut dyn Q3ServerBindings,
    ) -> Self {
        Self {
            identity,
            challenge,
            channel: Netchannel::new(ChannelRole::Server, qport, || 0),
            reliable: ServerReliableCommands::new(),
            source_state: SourceMessageState::new(|_| {}),
            snapshots,
            queued_messages: Vec::new(),
            phase: Q3ServerPhase::Connected,
            last_client_command: 0,
            last_client_command_string: String::new(),
            message_acknowledge: 0,
            delta_message: -1,
            gamestate_message_number: -1,
            last_user_command: WireUserCommand::default(),
            pure_authentic: false,
            got_pure_command: false,
            next_snapshot_time: 0,
            rate_delayed: false,
            reliable_sent: 0,
            next_reliable_time: 0,
            bindings,
        }
    }

    /// Accept one datagram through the channel (`receiveDatagram`).
    pub fn receive_datagram(&mut self, packet: &[u8]) -> Result<ChannelResult, Q3NetError> {
        self.bindings.assert_current();
        let result = self.channel.receive(packet, None);
        if !matches!(result, ChannelResult::Accepted { .. })
            || self.phase == Q3ServerPhase::Zombie
        {
            return Ok(result);
        }
        let plaintext = match &result {
            ChannelResult::Accepted { payload, .. } => {
                let lookup = |sequence: i32| self.reliable.lookup_masked(sequence);
                xor_client_message(payload, self.challenge, &lookup)?
            }
            _ => unreachable!("matched above"),
        };
        self.execute_message(ClientMessageReader::new(&plaintext)?)?;
        Ok(result)
    }

    /// Execute one decoded client message (`executeMessage`).
    pub fn execute_message(
        &mut self,
        mut reader: ClientMessageReader<'_>,
    ) -> Result<(), Q3NetError> {
        let server_id = self.bindings.server_id();
        self.bindings.assert_current();
        self.message_acknowledge = reader.prefix.message_acknowledge;
        if self.message_acknowledge < 0 {
            if self.bindings.debug_build() {
                self.bindings.drop_client("DEBUG: illegible client message")?;
            }
            return Ok(());
        }
        let header = reader.read_header()?;
        self.reliable.assign_acknowledgement(header.reliable_acknowledge);
        if self.reliable.acknowledge() < self.reliable.sequence() - 64 {
            if self.bindings.debug_build() {
                self.bindings.drop_client("DEBUG: illegible client message")?;
            }
            let sequence = self.reliable.sequence();
            self.reliable.assign_acknowledgement(sequence);
            return Ok(());
        }
        let server_now = self.bindings.server_id();
        if header.server_id != server_now
            && self.bindings.download_name().is_empty()
            && !self.last_client_command_string.contains("nextdl")
        {
            if header.server_id >= self.bindings.restarted_server_id()
                && header.server_id < server_now
            {
                return Ok(());
            }
            if self.message_acknowledge > self.gamestate_message_number {
                self.bindings.resend_gamestate()?;
            }
            return Ok(());
        }
        loop {
            self.bindings.assert_current();
            let part = match reader.next() {
                Ok(part) => part,
                Err(Q3NetError::ClientOpcode { .. }) => {
                    self.bindings.print("WARNING: bad command byte for client\n");
                    return Ok(());
                }
                Err(other) => return Err(other),
            };
            match part {
                ClientMessagePart::Eof => return Ok(()),
                ClientMessagePart::Command(command) => {
                    if self.last_client_command >= command.sequence {
                        continue;
                    }
                    if command.sequence > self.last_client_command.wrapping_add(1) {
                        self.bindings.drop_client("Lost reliable commands")?;
                        return Ok(());
                    }
                    let client_ok = self.bindings.client_running()
                        || self.phase != Q3ServerPhase::Active
                        || !self.bindings.flood_protect()
                        || self.bindings.time() >= self.next_reliable_time;
                    self.next_reliable_time = self.bindings.time().wrapping_add(1000);
                    if !self.bindings.command(&command, client_ok)? || !self.current(server_id)? {
                        return Ok(());
                    }
                    self.last_client_command = command.sequence;
                    self.last_client_command_string = stored_text(&command.text);
                    if self.phase == Q3ServerPhase::Zombie {
                        return Ok(());
                    }
                }
                ClientMessagePart::Movement { delta_message, .. } => {
                    self.delta_message = delta_message;
                    self.user_move(&mut reader)?;
                    return Ok(());
                }
            }
        }
    }

    fn current(&mut self, server_id: i32) -> Result<bool, Q3NetError> {
        if self.phase == Q3ServerPhase::Zombie {
            return Ok(false);
        }
        self.bindings.assert_current();
        if server_id != self.bindings.server_id() {
            return Err(Q3NetError::Protocol(
                "Q3 client callback belongs to a retired server world",
            ));
        }
        Ok(true)
    }

    fn user_move(&mut self, reader: &mut ClientMessageReader<'_>) -> Result<(), Q3NetError> {
        let server_id = self.bindings.server_id();
        let movement = {
            let lookup = |sequence: i32| self.reliable.lookup_masked(sequence);
            let context = ClientKeyContext {
                checksum_feed: self.bindings.checksum_feed(),
                server_command: &lookup,
            };
            match reader.read_movement(&context) {
                Ok(movement) => movement,
                Err(Q3NetError::ClientCommandCount { count }) => {
                    self.bindings.print(if count < 1 {
                        "cmdCount < 1\n"
                    } else {
                        "cmdCount > MAX_PACKET_USERCMDS\n"
                    });
                    return Ok(());
                }
                Err(other) => return Err(other),
            }
        };
        self.snapshots.frame_mut(self.message_acknowledge)?.message_acked =
            self.bindings.time();
        if self.bindings.pure() && !self.pure_authentic && !self.got_pure_command {
            if self.phase == Q3ServerPhase::Active {
                self.bindings.resend_gamestate()?;
            }
            return Ok(());
        }
        if self.phase == Q3ServerPhase::Primed {
            let first = movement.commands[0].clone();
            self.last_user_command = first.clone();
            self.phase = Q3ServerPhase::Active;
            self.bindings.enter_world(&first)?;
            if !self.current(server_id)? {
                return Ok(());
            }
        }
        if self.bindings.pure() && !self.pure_authentic {
            self.bindings.drop_client("Cannot validate pure client!")?;
            return Ok(());
        }
        if self.phase != Q3ServerPhase::Active {
            self.delta_message = -1;
            return Ok(());
        }
        let latest = movement
            .commands
            .last()
            .cloned()
            .ok_or(Q3NetError::Range("Missing decoded user command"))?;
        for command in &movement.commands {
            if command.server_time > latest.server_time
                || command.server_time <= self.last_user_command.server_time
            {
                continue;
            }
            self.last_user_command = command.clone();
            self.bindings.think(command)?;
            if !self.current(server_id)? {
                return Ok(());
            }
        }
        Ok(())
    }

    fn transmit(
        &mut self,
        writer: &mut Q3MsgWriter,
        rate: Q3ServerRate,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        let now = self.bindings.time();
        let outgoing = self.channel.outgoing_sequence();
        let frame = self.snapshots.frame_mut(outgoing)?;
        frame.message_size = writer.byte_length();
        frame.message_sent = now;
        frame.message_acked = -1;
        writer.write_byte(ServerOpcode::Eof as i32)?;
        let plaintext = writer.to_bytes().to_vec();
        if self.channel.has_unsent_fragments() {
            self.queued_messages.push(plaintext);
            self.channel.transmit_next_fragment(delivery)?;
        } else {
            let bytes = xor_server_message(
                &plaintext,
                self.challenge,
                outgoing,
                &self.last_client_command_string,
            );
            self.channel.begin_transmit(&bytes, delivery)?;
        }
        if rate.local || (rate.force_lan && rate.lan) {
            self.next_snapshot_time = now.wrapping_sub(1);
            return Ok(());
        }
        let mut interval =
            q3_rate_milliseconds(writer.byte_length() as i32, rate.rate, rate.max_rate)?;
        if interval < rate.snapshot_msec {
            interval = rate.snapshot_msec;
            self.rate_delayed = false;
        } else {
            self.rate_delayed = true;
        }
        self.next_snapshot_time = now.wrapping_add(interval);
        if self.phase != Q3ServerPhase::Active
            && self.bindings.download_name().is_empty()
            && self.next_snapshot_time < now.wrapping_add(1000)
        {
            self.next_snapshot_time = now.wrapping_add(1000);
        }
        Ok(())
    }

    /// Send the gamestate (`sendGamestate`).
    pub fn send_gamestate(
        &mut self,
        state: &Gamestate,
        rate: Q3ServerRate,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        self.phase = Q3ServerPhase::Primed;
        self.pure_authentic = false;
        self.got_pure_command = false;
        self.gamestate_message_number = self.channel.outgoing_sequence();
        let mut writer = Q3MsgWriter::new(MessageMode::Bitstream, 16384)?;
        let mut operations: Vec<ServerOperation> = self
            .reliable
            .pending()
            .iter()
            .map(|command| ServerOperation::Command {
                sequence: command.sequence,
                text: command.text.clone(),
            })
            .collect();
        let mut gamestate = state.clone();
        gamestate.command_sequence = self.reliable.sequence();
        gamestate.checksum_feed = self.bindings.checksum_feed();
        operations.push(ServerOperation::Gamestate(gamestate));
        let product = self.snapshots.product;
        let message_number = self.channel.outgoing_sequence();
        let baseline = |number: i32| Some((self.snapshots.baseline)(number));
        let history = |_number: i32| None;
        write_server_message(
            &mut writer,
            self.last_client_command,
            &operations,
            &ServerMessageContext {
                product,
                message_number,
                reliable_sequence: self.last_client_command,
                server_command_sequence: self.reliable.sequence(),
                parse_entities_number: 0,
                baseline: &baseline,
                history: &history,
            },
        )?;
        self.reliable_sent = self.reliable.sequence();
        self.transmit(&mut writer, rate, delivery)
    }

    /// Send one snapshot (`sendSnapshot`).
    pub fn send_snapshot(
        &mut self,
        server_flags: i32,
        rate: Q3ServerRate,
        delivery: &mut dyn ChannelDelivery,
        append_download: &mut dyn FnMut(&mut Q3MsgWriter),
    ) -> Result<(), Q3NetError> {
        if self.channel.has_unsent_fragments() {
            let interval = q3_rate_milliseconds(
                self.channel.remaining_unsent_bytes() as i32,
                rate.rate,
                rate.max_rate,
            )?;
            self.next_snapshot_time = self.bindings.time().wrapping_add(interval);
            self.transmit_next_fragment(delivery)?;
            return Ok(());
        }
        let mut writer = Q3MsgWriter::new(MessageMode::Bitstream, 16384)?;
        writer.write_long(self.last_client_command)?;
        for command in self.reliable.pending() {
            writer.write_byte(ServerOpcode::Command as i32)?;
            writer.write_long(command.sequence)?;
            writer.write_string(Some(&command.text))?;
        }
        self.reliable_sent = self.reliable.sequence();
        let outgoing = self.channel.outgoing_sequence();
        let active = self.phase == Q3ServerPhase::Active;
        let now = self.bindings.time();
        let flags =
            server_flags | i32::from(self.rate_delayed) | if active { 0 } else { 2 };
        self.snapshots.write(
            &mut writer,
            outgoing,
            self.delta_message,
            active,
            now,
            flags,
        )?;
        append_download(&mut writer);
        if writer.overflowed() {
            self.bindings.print("WARNING: msg overflowed\n");
            writer.clear();
        }
        self.transmit(&mut writer, rate, delivery)
    }

    /// Transmit the next fragment, dequeuing one queued message (`transmitNextFragment`).
    pub fn transmit_next_fragment(
        &mut self,
        delivery: &mut dyn ChannelDelivery,
    ) -> Result<(), Q3NetError> {
        if !self.channel.has_unsent_fragments() {
            return Err(Q3NetError::Protocol("No pending server fragments"));
        }
        self.channel.transmit_next_fragment(delivery)?;
        if !self.channel.has_unsent_fragments() && !self.queued_messages.is_empty() {
            let next = self.queued_messages.remove(0);
            let outgoing = self.channel.outgoing_sequence();
            let bytes =
                xor_server_message(&next, self.challenge, outgoing, &self.last_client_command_string);
            self.channel.begin_transmit(&bytes, delivery)?;
        }
        Ok(())
    }

    /// Verify a pure command (`verifyPure`).
    pub fn verify_pure(
        &mut self,
        server: &Q3PureServer,
        argv: &[String],
        send_rejected_snapshot: &mut dyn FnMut(),
    ) -> Result<Q3PureResult, Q3NetError> {
        let result = verify_q3_pure_command(server, argv)?;
        if matches!(result, Q3PureResult::Ignored(_)) {
            return Ok(result);
        }
        self.got_pure_command = true;
        self.pure_authentic = matches!(result, Q3PureResult::Authentic);
        if let Q3PureResult::Rejected(reason) = &result {
            self.next_snapshot_time = -1;
            self.phase = Q3ServerPhase::Active;
            send_rejected_snapshot();
            self.bindings.drop_client(reason)?;
        }
        Ok(result)
    }
}

