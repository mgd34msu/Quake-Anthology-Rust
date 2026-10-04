//! Port of Quake-Anthology-TS `src/network/q3/adapters.ts`
//!
//! `requireQ3Protocol` plus the `Q3UserCommand`, `Q3EntityState`,
//! `Q3PlayerState`, and `Q3Snapshot` contract/wire conversions. `Q3_PROTOCOL`
//! already lives as [`ProtocolIdentity::Q3`](crate::protocol::ProtocolIdentity)
//! and is reused, not duplicated. Wire state reuses [`WireUserCommand`],
//! [`Q3EntityState`], [`Q3PlayerState`], [`Q3Trajectory`], and [`Snapshot`];
//! the `src/contracts/protocol.ts` shapes are mirrored here at donor `f64`
//! precision because no Q3 contract shapes exist in `qa-core` or `qa-net`.
//! Storage vectors convert through `f32`, matching the wire records.

use thiserror::Error;

use crate::protocol::ProtocolIdentity;
use crate::q2_adapters::ContractVec3;
use crate::q3::WireUserCommand;
use crate::q3_net::{Q3EntityState, Q3PlayerSlots, Q3PlayerState, Q3Product, Q3Trajectory, Snapshot};

/// Adapter failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3AdapterError {
    /// A non-Q3 protocol reached the Q3 source codec.
    #[error("Quake 3 source codec requires protocol 68")]
    NotQ3,
    /// A raw trajectory tag has no contract kind.
    #[error("Raw Q3 trajectory tag {0} needs a mod-specific presentation binding")]
    TrajectoryTag(i32),
    /// A player-state slot list has the wrong length.
    #[error("Q3 wire needs {expected} source slots, received {got}")]
    SlotCount {
        /// Expected slots.
        expected: usize,
        /// Received slots.
        got: usize,
    },
}

/// Require the Quake III protocol identity (donor `requireQ3Protocol`).
pub fn require_q3_protocol(protocol: ProtocolIdentity) -> Result<ProtocolIdentity, Q3AdapterError> {
    if protocol == ProtocolIdentity::Q3 {
        Ok(protocol)
    } else {
        Err(Q3AdapterError::NotQ3)
    }
}

/// Contract three-vector (`Vec3` in `src/contracts/math.ts`, donor precision).
pub type Q3ContractVec3 = ContractVec3;

fn vec_to_wire(value: &Q3ContractVec3) -> [f32; 3] {
    [value.x as f32, value.y as f32, value.z as f32]
}

fn wire_to_vec(value: &[f32; 3]) -> Q3ContractVec3 {
    Q3ContractVec3 {
        x: f64::from(value[0]),
        y: f64::from(value[1]),
        z: f64::from(value[2]),
    }
}

/// Contract trajectory kind (donor `Q3Trajectory['kind']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3TrajectoryKind {
    /// Stationary.
    Stationary,
    /// Interpolate.
    Interpolate,
    /// Linear.
    Linear,
    /// Linear stop.
    LinearStop,
    /// Sine.
    Sine,
    /// Gravity.
    Gravity,
}

/// Map a raw trajectory tag to its contract kind (donor `trajectoryKind`).
fn trajectory_kind(tag: i32) -> Result<Q3TrajectoryKind, Q3AdapterError> {
    match tag {
        0 => Ok(Q3TrajectoryKind::Stationary),
        1 => Ok(Q3TrajectoryKind::Interpolate),
        2 => Ok(Q3TrajectoryKind::Linear),
        3 => Ok(Q3TrajectoryKind::LinearStop),
        4 => Ok(Q3TrajectoryKind::Sine),
        5 => Ok(Q3TrajectoryKind::Gravity),
        _ => Err(Q3AdapterError::TrajectoryTag(tag)),
    }
}

/// Map a contract kind to its raw trajectory tag (donor `trajectoryType`).
fn trajectory_tag(kind: Q3TrajectoryKind) -> i32 {
    match kind {
        Q3TrajectoryKind::Stationary => 0,
        Q3TrajectoryKind::Interpolate => 1,
        Q3TrajectoryKind::Linear => 2,
        Q3TrajectoryKind::LinearStop => 3,
        Q3TrajectoryKind::Sine => 4,
        Q3TrajectoryKind::Gravity => 5,
    }
}

/// Contract trajectory (donor `Q3Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ContractTrajectory {
    /// Trajectory kind.
    pub kind: Q3TrajectoryKind,
    /// Time in milliseconds.
    pub time_milliseconds: i32,
    /// Duration in milliseconds.
    pub duration_milliseconds: i32,
    /// Base.
    pub base: Q3ContractVec3,
    /// Delta.
    pub delta: Q3ContractVec3,
}

/// Convert a wire trajectory to its contract shape (donor `canonicalTrajectory`).
fn canonical_trajectory(value: &Q3Trajectory) -> Result<Q3ContractTrajectory, Q3AdapterError> {
    Ok(Q3ContractTrajectory {
        kind: trajectory_kind(value.trajectory_type)?,
        time_milliseconds: value.time,
        duration_milliseconds: value.duration,
        base: wire_to_vec(&value.base),
        delta: wire_to_vec(&value.delta),
    })
}

/// Convert a contract trajectory to wire storage (donor `wireTrajectory`).
fn wire_trajectory(value: &Q3ContractTrajectory) -> Q3Trajectory {
    Q3Trajectory {
        trajectory_type: trajectory_tag(value.kind),
        time: value.time_milliseconds,
        duration: value.duration_milliseconds,
        base: vec_to_wire(&value.base),
        delta: vec_to_wire(&value.delta),
    }
}

/// Contract user command (donor `Q3UserCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ContractUserCommand {
    /// Server time in milliseconds.
    pub server_time_milliseconds: i32,
    /// View angles as wire words.
    pub angle_words: [i32; 3],
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forward_move: i32,
    /// Right move.
    pub right_move: i32,
    /// Up move.
    pub up_move: i32,
}

/// Convert a wire user command to its contract shape (donor `toQ3UserCommand`).
#[must_use]
pub fn to_q3_user_command(value: &WireUserCommand) -> Q3ContractUserCommand {
    Q3ContractUserCommand {
        server_time_milliseconds: value.server_time,
        angle_words: [
            i32::from(value.angles[0]),
            i32::from(value.angles[1]),
            i32::from(value.angles[2]),
        ],
        buttons: i32::from(value.buttons),
        weapon: i32::from(value.weapon),
        forward_move: i32::from(value.moves[0]),
        right_move: i32::from(value.moves[1]),
        up_move: i32::from(value.moves[2]),
    }
}

/// Convert a contract user command to wire storage (donor `fromQ3UserCommand`).
#[must_use]
pub fn from_q3_user_command(value: &Q3ContractUserCommand) -> WireUserCommand {
    WireUserCommand {
        server_time: value.server_time_milliseconds,
        angles: [
            value.angle_words[0] as i16,
            value.angle_words[1] as i16,
            value.angle_words[2] as i16,
        ],
        moves: [value.forward_move as i8, value.right_move as i8, value.up_move as i8],
        buttons: value.buttons as u16,
        weapon: value.weapon as u8,
    }
}

/// Contract entity state (donor `Q3EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ContractEntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type.
    pub entity_type: i32,
    /// Entity flags.
    pub flags: i32,
    /// Position trajectory.
    pub position: Q3ContractTrajectory,
    /// Angular position trajectory.
    pub angular_position: Q3ContractTrajectory,
    /// Time in milliseconds.
    pub time_milliseconds: i32,
    /// Second time in milliseconds.
    pub time2_milliseconds: i32,
    /// Origin.
    pub origin: Q3ContractVec3,
    /// Second origin.
    pub origin2: Q3ContractVec3,
    /// Angles.
    pub angles: Q3ContractVec3,
    /// Second angles.
    pub angles2: Q3ContractVec3,
    /// Other entity number.
    pub other_entity_number: i32,
    /// Second other entity number.
    pub other_entity_number2: i32,
    /// Ground entity number.
    pub ground_entity_number: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Client number.
    pub client_number: i32,
    /// Frame.
    pub frame: i32,
    /// Solid.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parameter: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_animation: i32,
    /// Torso animation.
    pub torso_animation: i32,
    /// Generic value.
    pub generic1: i32,
}

/// Convert wire entity state to its contract shape (donor `toQ3EntityState`).
pub fn to_q3_entity_state(value: &Q3EntityState) -> Result<Q3ContractEntityState, Q3AdapterError> {
    Ok(Q3ContractEntityState {
        number: value.number,
        entity_type: value.e_type,
        flags: value.e_flags,
        position: canonical_trajectory(&value.pos)?,
        angular_position: canonical_trajectory(&value.apos)?,
        time_milliseconds: value.time,
        time2_milliseconds: value.time2,
        origin: wire_to_vec(&value.origin),
        origin2: wire_to_vec(&value.origin2),
        angles: wire_to_vec(&value.angles),
        angles2: wire_to_vec(&value.angles2),
        other_entity_number: value.other_entity_num,
        other_entity_number2: value.other_entity_num2,
        ground_entity_number: value.ground_entity_num,
        constant_light: value.constant_light,
        loop_sound: value.loop_sound,
        model_index: value.modelindex,
        model_index2: value.modelindex2,
        client_number: value.client_num,
        frame: value.frame,
        solid: value.solid,
        event: value.event,
        event_parameter: value.event_parm,
        powerups: value.powerups,
        weapon: value.weapon,
        legs_animation: value.legs_anim,
        torso_animation: value.torso_anim,
        generic1: value.generic1,
    })
}

/// Convert a contract entity state to wire storage (donor `fromQ3EntityState`).
#[must_use]
pub fn from_q3_entity_state(value: &Q3ContractEntityState) -> Q3EntityState {
    Q3EntityState {
        number: value.number,
        e_type: value.entity_type,
        e_flags: value.flags,
        pos: wire_trajectory(&value.position),
        apos: wire_trajectory(&value.angular_position),
        time: value.time_milliseconds,
        time2: value.time2_milliseconds,
        origin: vec_to_wire(&value.origin),
        origin2: vec_to_wire(&value.origin2),
        angles: vec_to_wire(&value.angles),
        angles2: vec_to_wire(&value.angles2),
        other_entity_num: value.other_entity_number,
        other_entity_num2: value.other_entity_number2,
        ground_entity_num: value.ground_entity_number,
        constant_light: value.constant_light,
        loop_sound: value.loop_sound,
        modelindex: value.model_index,
        modelindex2: value.model_index2,
        client_num: value.client_number,
        frame: value.frame,
        solid: value.solid,
        event: value.event,
        event_parm: value.event_parameter,
        powerups: value.powerups,
        weapon: value.weapon,
        legs_anim: value.legs_animation,
        torso_anim: value.torso_animation,
        generic1: value.generic1,
    }
}

/// Contract player state (donor `Q3PlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ContractPlayerState {
    /// Command time in milliseconds.
    pub command_time_milliseconds: i32,
    /// Movement type.
    pub movement_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub movement_flags: i32,
    /// Movement time in milliseconds.
    pub movement_time_milliseconds: i32,
    /// Origin.
    pub origin: Q3ContractVec3,
    /// Velocity.
    pub velocity: Q3ContractVec3,
    /// Weapon time in milliseconds.
    pub weapon_time_milliseconds: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles as wire words.
    pub delta_angle_words: [i32; 3],
    /// Ground entity number.
    pub ground_entity_number: i32,
    /// Legs timer in milliseconds.
    pub legs_timer_milliseconds: i32,
    /// Legs animation.
    pub legs_animation: i32,
    /// Torso timer in milliseconds.
    pub torso_timer_milliseconds: i32,
    /// Torso animation.
    pub torso_animation: i32,
    /// Movement direction.
    pub movement_direction: i32,
    /// Grapple point.
    pub grapple_point: Q3ContractVec3,
    /// Entity flags.
    pub flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Events.
    pub events: [i32; 2],
    /// Event parameters.
    pub event_parameters: [i32; 2],
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parameter: i32,
    /// External event time in milliseconds.
    pub external_event_time_milliseconds: i32,
    /// Client number.
    pub client_number: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub view_angles: Q3ContractVec3,
    /// View height.
    pub view_height: i32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: Vec<i32>,
    /// Persistent data.
    pub persistent: Vec<i32>,
    /// Powerups.
    pub powerups: Vec<i32>,
    /// Ammo.
    pub ammo: Vec<i32>,
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jump_pad_entity: i32,
    /// Ping in milliseconds.
    pub ping_milliseconds: i32,
    /// Movement frame count.
    pub movement_frame_count: i32,
    /// Jump-pad frame.
    pub jump_pad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

/// Copy the 16 wire slots (donor `slots.copy()`).
fn copy_slots(slots: &Q3PlayerSlots) -> Vec<i32> {
    (0..16)
        .map(|index| slots.get(index).expect("player state holds 16 slots"))
        .collect()
}

/// Fill wire slots from a contract list (donor `assignSlots`).
fn assign_slots(slots: &mut Q3PlayerSlots, values: &[i32]) -> Result<(), Q3AdapterError> {
    if values.len() != 16 {
        return Err(Q3AdapterError::SlotCount {
            expected: 16,
            got: values.len(),
        });
    }
    for (index, value) in values.iter().enumerate() {
        slots.set(index, *value).expect("slot length is checked");
    }
    Ok(())
}

/// Convert wire player state to its contract shape (donor `toQ3PlayerState`).
#[must_use]
pub fn to_q3_player_state(value: &Q3PlayerState) -> Q3ContractPlayerState {
    Q3ContractPlayerState {
        command_time_milliseconds: value.command_time,
        movement_type: value.pm_type,
        bob_cycle: value.bob_cycle,
        movement_flags: value.pm_flags,
        movement_time_milliseconds: value.pm_time,
        origin: wire_to_vec(&value.origin),
        velocity: wire_to_vec(&value.velocity),
        weapon_time_milliseconds: value.weapon_time,
        gravity: value.gravity,
        speed: value.speed,
        delta_angle_words: [
            value.delta_angles[0] as i32,
            value.delta_angles[1] as i32,
            value.delta_angles[2] as i32,
        ],
        ground_entity_number: value.ground_entity_num,
        legs_timer_milliseconds: value.legs_timer,
        legs_animation: value.legs_anim,
        torso_timer_milliseconds: value.torso_timer,
        torso_animation: value.torso_anim,
        movement_direction: value.movement_dir,
        grapple_point: wire_to_vec(&value.grapple_point),
        flags: value.e_flags,
        event_sequence: value.event_sequence,
        events: value.events,
        event_parameters: value.event_parms,
        external_event: value.external_event,
        external_event_parameter: value.external_event_parm,
        external_event_time_milliseconds: value.external_event_time,
        client_number: value.client_num,
        weapon: value.weapon,
        weapon_state: value.weapon_state,
        view_angles: wire_to_vec(&value.viewangles),
        view_height: value.viewheight,
        damage_event: value.damage_event,
        damage_yaw: value.damage_yaw,
        damage_pitch: value.damage_pitch,
        damage_count: value.damage_count,
        stats: copy_slots(&value.stats),
        persistent: copy_slots(&value.persistant),
        powerups: copy_slots(&value.powerups),
        ammo: copy_slots(&value.ammo),
        generic1: value.generic1,
        loop_sound: value.loop_sound,
        jump_pad_entity: value.jumppad_ent,
        ping_milliseconds: value.ping,
        movement_frame_count: value.pmove_framecount,
        jump_pad_frame: value.jumppad_frame,
        entity_event_sequence: value.entity_event_sequence,
    }
}

/// Convert a contract player state to wire storage (donor `fromQ3PlayerState`).
pub fn from_q3_player_state(
    value: &Q3ContractPlayerState,
    product: Q3Product,
) -> Result<Q3PlayerState, Q3AdapterError> {
    let mut result = Q3PlayerState::new(product);
    result.command_time = value.command_time_milliseconds;
    result.pm_type = value.movement_type;
    result.bob_cycle = value.bob_cycle;
    result.pm_flags = value.movement_flags;
    result.pm_time = value.movement_time_milliseconds;
    result.origin = vec_to_wire(&value.origin);
    result.velocity = vec_to_wire(&value.velocity);
    result.weapon_time = value.weapon_time_milliseconds;
    result.gravity = value.gravity;
    result.speed = value.speed;
    result.ground_entity_num = value.ground_entity_number;
    result.legs_timer = value.legs_timer_milliseconds;
    result.legs_anim = value.legs_animation;
    result.torso_timer = value.torso_timer_milliseconds;
    result.torso_anim = value.torso_animation;
    result.movement_dir = value.movement_direction;
    result.grapple_point = vec_to_wire(&value.grapple_point);
    result.e_flags = value.flags;
    result.event_sequence = value.event_sequence;
    result.events = value.events;
    result.event_parms = value.event_parameters;
    result.external_event = value.external_event;
    result.external_event_parm = value.external_event_parameter;
    result.external_event_time = value.external_event_time_milliseconds;
    result.client_num = value.client_number;
    result.weapon = value.weapon;
    result.weapon_state = value.weapon_state;
    result.viewangles = vec_to_wire(&value.view_angles);
    result.viewheight = value.view_height;
    result.damage_event = value.damage_event;
    result.damage_yaw = value.damage_yaw;
    result.damage_pitch = value.damage_pitch;
    result.damage_count = value.damage_count;
    result.generic1 = value.generic1;
    result.loop_sound = value.loop_sound;
    result.jumppad_ent = value.jump_pad_entity;
    result.ping = value.ping_milliseconds;
    result.pmove_framecount = value.movement_frame_count;
    result.jumppad_frame = value.jump_pad_frame;
    result.entity_event_sequence = value.entity_event_sequence;
    result.delta_angles = [
        value.delta_angle_words[0] as f32,
        value.delta_angle_words[1] as f32,
        value.delta_angle_words[2] as f32,
    ];
    assign_slots(&mut result.stats, &value.stats)?;
    assign_slots(&mut result.persistant, &value.persistent)?;
    assign_slots(&mut result.powerups, &value.powerups)?;
    assign_slots(&mut result.ammo, &value.ammo)?;
    Ok(result)
}

/// Contract snapshot (donor `Q3Snapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ContractSnapshot {
    /// Protocol identity (donor `Q3_PROTOCOL`).
    pub protocol: ProtocolIdentity,
    /// Snapshot flags.
    pub flags: i32,
    /// Ping in milliseconds.
    pub ping_milliseconds: i32,
    /// Server time in milliseconds.
    pub server_time_milliseconds: i32,
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// Player state.
    pub player: Q3ContractPlayerState,
    /// Entities.
    pub entities: Vec<Q3ContractEntityState>,
    /// Server command count.
    pub server_command_count: i32,
    /// Server command sequence.
    pub server_command_sequence: i32,
}

/// Convert a wire snapshot to its contract shape (donor `toQ3Snapshot`).
pub fn to_q3_snapshot(
    value: &Snapshot,
    ping_milliseconds: i32,
    server_command_count: i32,
) -> Result<Q3ContractSnapshot, Q3AdapterError> {
    Ok(Q3ContractSnapshot {
        protocol: ProtocolIdentity::Q3,
        flags: value.flags,
        ping_milliseconds,
        server_time_milliseconds: value.server_time,
        area_mask: value.area_mask.clone(),
        player: to_q3_player_state(&value.player_state),
        entities: value
            .entities
            .iter()
            .map(to_q3_entity_state)
            .collect::<Result<Vec<_>, _>>()?,
        server_command_count,
        server_command_sequence: value.server_command_number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_player() -> Q3ContractPlayerState {
        Q3ContractPlayerState {
            command_time_milliseconds: 50,
            movement_type: 0,
            bob_cycle: 1,
            movement_flags: 2,
            movement_time_milliseconds: 3,
            origin: Q3ContractVec3 { x: 1.0, y: 2.0, z: 3.0 },
            velocity: Q3ContractVec3 { x: 4.0, y: 5.0, z: 6.0 },
            weapon_time_milliseconds: 7,
            gravity: 800,
            speed: 320,
            delta_angle_words: [9, 10, 11],
            ground_entity_number: 12,
            legs_timer_milliseconds: 13,
            legs_animation: 14,
            torso_timer_milliseconds: 15,
            torso_animation: 16,
            movement_direction: 17,
            grapple_point: Q3ContractVec3 { x: 7.0, y: 8.0, z: 9.0 },
            flags: 18,
            event_sequence: 19,
            events: [20, 21],
            event_parameters: [22, 23],
            external_event: 24,
            external_event_parameter: 25,
            external_event_time_milliseconds: 26,
            client_number: 27,
            weapon: 28,
            weapon_state: 29,
            view_angles: Q3ContractVec3 {
                x: 30.0,
                y: 31.0,
                z: 32.0,
            },
            view_height: 33,
            damage_event: 34,
            damage_yaw: 35,
            damage_pitch: 36,
            damage_count: 37,
            stats: (0..16).collect(),
            persistent: (16..32).collect(),
            powerups: (32..48).collect(),
            ammo: (48..64).collect(),
            generic1: 38,
            loop_sound: 39,
            jump_pad_entity: 40,
            ping_milliseconds: 41,
            movement_frame_count: 42,
            jump_pad_frame: 43,
            entity_event_sequence: 44,
        }
    }

    #[test]
    fn protocol_gate_accepts_only_q3() {
        assert_eq!(require_q3_protocol(ProtocolIdentity::Q3).unwrap(), ProtocolIdentity::Q3);
        assert_eq!(
            require_q3_protocol(ProtocolIdentity::Q2Classic),
            Err(Q3AdapterError::NotQ3)
        );
    }

    #[test]
    fn user_commands_round_trip() {
        let wire = WireUserCommand {
            server_time: 50,
            angles: [100, -200, 300],
            moves: [1, -2, 3],
            buttons: 33,
            weapon: 7,
        };
        let contract = to_q3_user_command(&wire);
        assert_eq!(contract.angle_words, [100, -200, 300]);
        assert_eq!(from_q3_user_command(&contract), wire);
    }

    #[test]
    fn entities_round_trip() {
        let wire = Q3EntityState {
            number: 5,
            e_type: 2,
            pos: Q3Trajectory {
                trajectory_type: 5,
                ..Default::default()
            },
            apos: Q3Trajectory {
                trajectory_type: 1,
                ..Default::default()
            },
            origin: [1.0, 2.0, 3.0],
            ..Default::default()
        };
        let contract = to_q3_entity_state(&wire).unwrap();
        assert_eq!(contract.position.kind, Q3TrajectoryKind::Gravity);
        assert_eq!(contract.angular_position.kind, Q3TrajectoryKind::Interpolate);
        assert_eq!(from_q3_entity_state(&contract), wire);
    }

    #[test]
    fn unknown_trajectory_tag_fails() {
        let mut wire = Q3EntityState::default();
        wire.pos.trajectory_type = 9;
        assert_eq!(to_q3_entity_state(&wire), Err(Q3AdapterError::TrajectoryTag(9)));
    }

    #[test]
    fn players_round_trip() {
        let contract = contract_player();
        let wire = from_q3_player_state(&contract, Q3Product::Base).unwrap();
        assert_eq!(wire.product, Q3Product::Base);
        assert_eq!(wire.delta_angles, [9.0, 10.0, 11.0]);
        assert_eq!(to_q3_player_state(&wire), contract);
    }

    #[test]
    fn short_slot_list_fails() {
        let mut contract = contract_player();
        contract.ammo.pop();
        assert_eq!(
            from_q3_player_state(&contract, Q3Product::Base),
            Err(Q3AdapterError::SlotCount { expected: 16, got: 15 })
        );
    }

    #[test]
    fn snapshots_carry_protocol_and_commands() {
        let wire = Snapshot {
            message_number: 2,
            server_time: 50,
            delta_number: 0,
            flags: 1,
            server_command_number: 7,
            parse_entities_number: 0,
            area_mask: vec![1, 2],
            player_state: Q3PlayerState::new(Q3Product::Base),
            entities: vec![Q3EntityState::default()],
        };
        let contract = to_q3_snapshot(&wire, 42, 9).unwrap();
        assert_eq!(contract.protocol, ProtocolIdentity::Q3);
        assert_eq!(contract.ping_milliseconds, 42);
        assert_eq!(contract.server_command_count, 9);
        assert_eq!(contract.server_command_sequence, 7);
        assert_eq!(contract.entities.len(), 1);
    }
}
