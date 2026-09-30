//! Client snapshot, game-state, and user-command record layouts.
//!
//! Provenance: `src/compat/qvm/client-state-record.ts` (Q3 `cl_cgame.c`
//! snapshot copies and `q_shared.h` guest ABI). The [`PlayerStateFields`],
//! [`EntityStateFields`], and [`Trajectory`] types plus their read/write
//! layouts are local mirrors of `entity-record.ts` and `player-record.ts`
//! (owned by other workers); legacy translation reuses
//! [`super::legacy_presentation`].

use qa_core::math::Vec3;

use super::client_state::{AbiProfile, GameStateRecord, SyscallMemory, WireUserCommand};
use super::legacy_presentation::{qvm_configstring, qvm_entity_type, qvm_event, qvm_powerup_bits, qvm_powerups};
use crate::error::GuestError;

/// Byte length of `gameState_t`.
pub const QVM_GAME_STATE_BYTES: usize = 20100;
/// Byte length of modern `snapshot_t`.
pub const QVM_SNAPSHOT_BYTES: usize = 53772;
/// Byte length of legacy `snapshot_t`.
pub const QVM_LEGACY_SNAPSHOT_BYTES: usize = 52724;
/// Byte length of `usercmd_t`.
pub const QVM_USER_COMMAND_BYTES: usize = 24;
/// Byte length of modern `playerState_t`.
pub const QVM_PLAYER_STATE_BYTES: usize = 468;
/// Byte length of legacy `playerState_t`.
pub const QVM_LEGACY_PLAYER_STATE_BYTES: usize = 444;
/// Byte length of modern `entityState_t`.
pub const QVM_ENTITY_STATE_BYTES: usize = 208;
/// Byte length of legacy `entityState_t`.
pub const QVM_LEGACY_ENTITY_STATE_BYTES: usize = 204;

/// Snapshot byte length for a profile.
#[must_use]
pub const fn qvm_snapshot_bytes(profile: AbiProfile) -> usize {
    match profile {
        AbiProfile::Modern => QVM_SNAPSHOT_BYTES,
        AbiProfile::Legacy => QVM_LEGACY_SNAPSHOT_BYTES,
    }
}

/// Player-state byte length for a profile.
#[must_use]
pub const fn qvm_player_state_bytes(profile: AbiProfile) -> usize {
    match profile {
        AbiProfile::Modern => QVM_PLAYER_STATE_BYTES,
        AbiProfile::Legacy => QVM_LEGACY_PLAYER_STATE_BYTES,
    }
}

/// Entity-state byte length for a profile.
#[must_use]
pub const fn qvm_entity_state_bytes(profile: AbiProfile) -> usize {
    match profile {
        AbiProfile::Modern => QVM_ENTITY_STATE_BYTES,
        AbiProfile::Legacy => QVM_LEGACY_ENTITY_STATE_BYTES,
    }
}

/// User-command field layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserCommandLayout {
    /// Buttons field offset and width.
    pub buttons_offset: usize,
    /// Whether buttons occupy a single byte (legacy) or word (modern).
    pub buttons_byte: bool,
    /// Angles base offset.
    pub angles: usize,
    /// Weapon offset.
    pub weapon: usize,
    /// Forward-move offset.
    pub forward: usize,
    /// Right-move offset.
    pub right: usize,
    /// Up-move offset.
    pub up: usize,
}

/// User-command layout for a profile.
#[must_use]
pub const fn qvm_user_command_layout(profile: AbiProfile) -> UserCommandLayout {
    match profile {
        AbiProfile::Modern => UserCommandLayout {
            buttons_offset: 16,
            buttons_byte: false,
            angles: 4,
            weapon: 20,
            forward: 21,
            right: 22,
            up: 23,
        },
        AbiProfile::Legacy => UserCommandLayout {
            buttons_offset: 4,
            buttons_byte: true,
            angles: 8,
            weapon: 5,
            forward: 20,
            right: 21,
            up: 22,
        },
    }
}

/// Trajectory record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trajectory {
    /// Trajectory type.
    pub trajectory_type: i32,
    /// Start time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta velocity.
    pub delta: Vec3,
}

/// Entity-state fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityStateFields {
    /// Entity number.
    pub number: i32,
    /// Entity type.
    pub entity_type: i32,
    /// Entity flags.
    pub flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angular trajectory.
    pub apos: Trajectory,
    /// Time.
    pub time: i32,
    /// Second time.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Second origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Second angles.
    pub angles2: Vec3,
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
    /// Solid encoding.
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
    /// Generic value (modern only; legacy writes reject nonzero).
    pub generic1: i32,
}

/// Player-state fields.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerStateFields {
    /// Command time in milliseconds.
    pub command_time_ms: i32,
    /// Movement type.
    pub movement_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub movement_flags: i32,
    /// Movement time in milliseconds.
    pub movement_time_ms: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Weapon time in milliseconds.
    pub weapon_time_ms: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angle words.
    pub delta_angles: [i32; 3],
    /// Ground entity number.
    pub ground_entity_number: i32,
    /// Legs timer in milliseconds.
    pub legs_timer_ms: i32,
    /// Legs animation.
    pub legs_animation: i32,
    /// Torso timer in milliseconds.
    pub torso_timer_ms: i32,
    /// Torso animation.
    pub torso_animation: i32,
    /// Movement direction.
    pub movement_direction: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Flags.
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
    pub external_event_time_ms: i32,
    /// Client number.
    pub client_number: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub view_angles: Vec3,
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
    /// Stats slots.
    pub stats: [i32; 16],
    /// Persistent slots.
    pub persistent: [i32; 16],
    /// Powerup slots.
    pub powerups: [i32; 16],
    /// Ammo slots.
    pub ammo: [i32; 16],
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jump_pad_entity: i32,
    /// Ping in milliseconds.
    pub ping_ms: i32,
    /// Movement frame count.
    pub movement_frame_count: i32,
    /// Jump-pad frame.
    pub jump_pad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

/// Source snapshot with owned entity records.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSnapshot {
    /// Snapshot number.
    pub number: i32,
    /// Server time.
    pub server_time: i32,
    /// Snapshot flags.
    pub flags: i32,
    /// Area mask.
    pub area_mask: [u8; 32],
    /// Player state.
    pub player_state: PlayerStateFields,
    /// Entity states (at most 256).
    pub entities: Vec<EntityStateFields>,
    /// Server-command sequence.
    pub server_command_sequence: i32,
}

fn require(view: &[u8], size: usize) -> Result<(), GuestError> {
    if view.len() < size {
        return Err(GuestError::invalid(format!("QVM client record requires {size} bytes")));
    }
    Ok(())
}

fn read_i32(view: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(view[offset..offset + 4].try_into().expect("checked record"))
}

fn read_f32(view: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(view[offset..offset + 4].try_into().expect("checked record"))
}

fn read_vec3(view: &[u8], offset: usize) -> Vec3 {
    Vec3 {
        x: read_f32(view, offset),
        y: read_f32(view, offset + 4),
        z: read_f32(view, offset + 8),
    }
}

fn write_i32(view: &mut [u8], offset: usize, value: i32) {
    view[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(view: &mut [u8], offset: usize, value: f32) {
    view[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_vec3(view: &mut [u8], offset: usize, value: &Vec3) {
    write_f32(view, offset, value.x);
    write_f32(view, offset + 4, value.y);
    write_f32(view, offset + 8, value.z);
}

fn write_trajectory(view: &mut [u8], offset: usize, value: &Trajectory) {
    write_i32(view, offset, value.trajectory_type);
    write_i32(view, offset + 4, value.time);
    write_i32(view, offset + 8, value.duration);
    write_vec3(view, offset + 12, &value.base);
    write_vec3(view, offset + 24, &value.delta);
}

fn read_trajectory(view: &[u8], offset: usize) -> Trajectory {
    Trajectory {
        trajectory_type: read_i32(view, offset),
        time: read_i32(view, offset + 4),
        duration: read_i32(view, offset + 8),
        base: read_vec3(view, offset + 12),
        delta: read_vec3(view, offset + 24),
    }
}

fn read_slots(view: &[u8], offset: usize) -> [i32; 16] {
    core::array::from_fn(|index| read_i32(view, offset + index * 4))
}

fn write_slots(view: &mut [u8], offset: usize, slots: &[i32]) {
    for (index, value) in slots.iter().enumerate() {
        write_i32(view, offset + index * 4, *value);
    }
}

/// Read an entity-state record with presentation translation.
pub fn read_entity_state(view: &[u8], profile: AbiProfile) -> Result<EntityStateFields, GuestError> {
    let mut state = read_source_entity_state(view, profile)?;
    state.entity_type = qvm_entity_type(state.entity_type, profile, false)?;
    state.event = qvm_event(state.event, profile, false)?;
    state.powerups = qvm_powerup_bits(state.powerups, profile)?;
    Ok(state)
}

/// Read an entity-state record without presentation translation.
pub fn read_source_entity_state(view: &[u8], profile: AbiProfile) -> Result<EntityStateFields, GuestError> {
    require(view, qvm_entity_state_bytes(profile))?;
    Ok(EntityStateFields {
        number: read_i32(view, 0),
        entity_type: read_i32(view, 4),
        flags: read_i32(view, 8),
        pos: read_trajectory(view, 12),
        apos: read_trajectory(view, 48),
        time: read_i32(view, 84),
        time2: read_i32(view, 88),
        origin: read_vec3(view, 92),
        origin2: read_vec3(view, 104),
        angles: read_vec3(view, 116),
        angles2: read_vec3(view, 128),
        other_entity_num: read_i32(view, 140),
        other_entity_num2: read_i32(view, 144),
        ground_entity_num: read_i32(view, 148),
        constant_light: read_i32(view, 152),
        loop_sound: read_i32(view, 156),
        modelindex: read_i32(view, 160),
        modelindex2: read_i32(view, 164),
        client_num: read_i32(view, 168),
        frame: read_i32(view, 172),
        solid: read_i32(view, 176),
        event: read_i32(view, 180),
        event_parm: read_i32(view, 184),
        powerups: read_i32(view, 188),
        weapon: read_i32(view, 192),
        legs_anim: read_i32(view, 196),
        torso_anim: read_i32(view, 200),
        generic1: if profile.is_modern() { read_i32(view, 204) } else { 0 },
    })
}

/// Write an entity-state record with presentation translation.
pub fn write_entity_state(view: &mut [u8], state: &EntityStateFields, profile: AbiProfile) -> Result<(), GuestError> {
    let translated = EntityStateFields {
        entity_type: qvm_entity_type(state.entity_type, profile, true)?,
        event: qvm_event(state.event, profile, true)?,
        powerups: qvm_powerup_bits(state.powerups, profile)?,
        ..state.clone()
    };
    write_source_entity_state(view, &translated, profile)
}

/// Write an entity-state record without presentation translation.
pub fn write_source_entity_state(
    view: &mut [u8],
    state: &EntityStateFields,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    require(view, qvm_entity_state_bytes(profile))?;
    if !profile.is_modern() && state.generic1 != 0 {
        return Err(GuestError::invalid("Legacy QVM entity has no generic1 field"));
    }
    write_i32(view, 0, state.number);
    write_i32(view, 4, state.entity_type);
    write_i32(view, 8, state.flags);
    write_trajectory(view, 12, &state.pos);
    write_trajectory(view, 48, &state.apos);
    write_i32(view, 84, state.time);
    write_i32(view, 88, state.time2);
    write_vec3(view, 92, &state.origin);
    write_vec3(view, 104, &state.origin2);
    write_vec3(view, 116, &state.angles);
    write_vec3(view, 128, &state.angles2);
    write_i32(view, 140, state.other_entity_num);
    write_i32(view, 144, state.other_entity_num2);
    write_i32(view, 148, state.ground_entity_num);
    write_i32(view, 152, state.constant_light);
    write_i32(view, 156, state.loop_sound);
    write_i32(view, 160, state.modelindex);
    write_i32(view, 164, state.modelindex2);
    write_i32(view, 168, state.client_num);
    write_i32(view, 172, state.frame);
    write_i32(view, 176, state.solid);
    write_i32(view, 180, state.event);
    write_i32(view, 184, state.event_parm);
    write_i32(view, 188, state.powerups);
    write_i32(view, 192, state.weapon);
    write_i32(view, 196, state.legs_anim);
    write_i32(view, 200, state.torso_anim);
    if profile.is_modern() {
        write_i32(view, 204, state.generic1);
    }
    Ok(())
}

/// Read a player-state record with presentation translation.
pub fn read_player_state(view: &[u8], profile: AbiProfile) -> Result<PlayerStateFields, GuestError> {
    let mut state = read_source_player_state(view, profile)?;
    state.events = [
        qvm_event(state.events[0], profile, false)?,
        qvm_event(state.events[1], profile, false)?,
    ];
    state.external_event = qvm_event(state.external_event, profile, false)?;
    state.persistent = super::legacy_presentation::qvm_persistent(&state.persistent, profile);
    state.powerups = qvm_powerups(&state.powerups, profile)?;
    Ok(state)
}

/// Read a player-state record without presentation translation.
pub fn read_source_player_state(view: &[u8], profile: AbiProfile) -> Result<PlayerStateFields, GuestError> {
    require(view, qvm_player_state_bytes(profile))?;
    let modern = profile.is_modern();
    Ok(PlayerStateFields {
        command_time_ms: read_i32(view, 0),
        movement_type: read_i32(view, 4),
        bob_cycle: read_i32(view, 8),
        movement_flags: read_i32(view, 12),
        movement_time_ms: read_i32(view, 16),
        origin: read_vec3(view, 20),
        velocity: read_vec3(view, 32),
        weapon_time_ms: read_i32(view, 44),
        gravity: read_i32(view, 48),
        speed: read_i32(view, 52),
        delta_angles: [read_i32(view, 56), read_i32(view, 60), read_i32(view, 64)],
        ground_entity_number: read_i32(view, 68),
        legs_timer_ms: read_i32(view, 72),
        legs_animation: read_i32(view, 76),
        torso_timer_ms: read_i32(view, 80),
        torso_animation: read_i32(view, 84),
        movement_direction: read_i32(view, 88),
        grapple_point: read_vec3(view, 92),
        flags: read_i32(view, 104),
        event_sequence: read_i32(view, 108),
        events: [read_i32(view, 112), read_i32(view, 116)],
        event_parameters: [read_i32(view, 120), read_i32(view, 124)],
        external_event: read_i32(view, 128),
        external_event_parameter: read_i32(view, 132),
        external_event_time_ms: read_i32(view, 136),
        client_number: read_i32(view, 140),
        weapon: read_i32(view, 144),
        weapon_state: read_i32(view, 148),
        view_angles: read_vec3(view, 152),
        view_height: read_i32(view, 164),
        damage_event: read_i32(view, 168),
        damage_yaw: read_i32(view, 172),
        damage_pitch: read_i32(view, 176),
        damage_count: read_i32(view, 180),
        stats: read_slots(view, 184),
        persistent: read_slots(view, 248),
        powerups: read_slots(view, 312),
        ammo: read_slots(view, 376),
        generic1: if modern { read_i32(view, 440) } else { 0 },
        loop_sound: if modern { read_i32(view, 444) } else { 0 },
        jump_pad_entity: if modern { read_i32(view, 448) } else { 0 },
        ping_ms: read_i32(view, if modern { 452 } else { 440 }),
        movement_frame_count: if modern { read_i32(view, 456) } else { 0 },
        jump_pad_frame: if modern { read_i32(view, 460) } else { 0 },
        entity_event_sequence: if modern { read_i32(view, 464) } else { 0 },
    })
}

/// Write a player-state record with presentation translation.
pub fn write_player_state(view: &mut [u8], state: &PlayerStateFields, profile: AbiProfile) -> Result<(), GuestError> {
    let mut persistent = state.persistent;
    if !profile.is_modern() {
        let mut mapped = [0i32; 16];
        for index in [0, 1, 2, 3, 4, 8, 9, 10] {
            mapped[index] = state.persistent[index];
        }
        mapped[7] = state.persistent[6];
        mapped[11] = state.persistent[13];
        persistent = mapped;
    }
    let translated = PlayerStateFields {
        events: [
            qvm_event(state.events[0], profile, true)?,
            qvm_event(state.events[1], profile, true)?,
        ],
        external_event: qvm_event(state.external_event, profile, true)?,
        powerups: qvm_powerups(&state.powerups, profile)?,
        persistent,
        ..state.clone()
    };
    write_source_player_state(view, &translated, profile)
}

/// Write a player-state record without presentation translation.
pub fn write_source_player_state(
    view: &mut [u8],
    state: &PlayerStateFields,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    require(view, qvm_player_state_bytes(profile))?;
    write_i32(view, 0, state.command_time_ms);
    write_i32(view, 4, state.movement_type);
    write_i32(view, 8, state.bob_cycle);
    write_i32(view, 12, state.movement_flags);
    write_i32(view, 16, state.movement_time_ms);
    write_vec3(view, 20, &state.origin);
    write_vec3(view, 32, &state.velocity);
    write_i32(view, 44, state.weapon_time_ms);
    write_i32(view, 48, state.gravity);
    write_i32(view, 52, state.speed);
    write_i32(view, 56, state.delta_angles[0]);
    write_i32(view, 60, state.delta_angles[1]);
    write_i32(view, 64, state.delta_angles[2]);
    write_i32(view, 68, state.ground_entity_number);
    write_i32(view, 72, state.legs_timer_ms);
    write_i32(view, 76, state.legs_animation);
    write_i32(view, 80, state.torso_timer_ms);
    write_i32(view, 84, state.torso_animation);
    write_i32(view, 88, state.movement_direction);
    write_vec3(view, 92, &state.grapple_point);
    write_i32(view, 104, state.flags);
    write_i32(view, 108, state.event_sequence);
    write_slots(view, 112, &state.events);
    write_slots(view, 120, &state.event_parameters);
    write_i32(view, 128, state.external_event);
    write_i32(view, 132, state.external_event_parameter);
    write_i32(view, 136, state.external_event_time_ms);
    write_i32(view, 140, state.client_number);
    write_i32(view, 144, state.weapon);
    write_i32(view, 148, state.weapon_state);
    write_vec3(view, 152, &state.view_angles);
    write_i32(view, 164, state.view_height);
    write_i32(view, 168, state.damage_event);
    write_i32(view, 172, state.damage_yaw);
    write_i32(view, 176, state.damage_pitch);
    write_i32(view, 180, state.damage_count);
    write_slots(view, 184, &state.stats);
    write_slots(view, 248, &state.persistent);
    write_slots(view, 312, &state.powerups);
    write_slots(view, 376, &state.ammo);
    if !profile.is_modern() {
        write_i32(view, 440, state.ping_ms);
        return Ok(());
    }
    write_i32(view, 440, state.generic1);
    write_i32(view, 444, state.loop_sound);
    write_i32(view, 448, state.jump_pad_entity);
    write_i32(view, 452, state.ping_ms);
    write_i32(view, 456, state.movement_frame_count);
    write_i32(view, 460, state.jump_pad_frame);
    write_i32(view, 464, state.entity_event_sequence);
    Ok(())
}

/// Write a game-state record with configstring index translation.
pub fn write_game_state(
    memory: &mut SyscallMemory,
    word: i32,
    state: &GameStateRecord,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    write_game_state_inner(memory, word, state, Some(profile))
}

/// Write a game-state record with identity configstring indices.
pub fn write_source_game_state(
    memory: &mut SyscallMemory,
    word: i32,
    state: &GameStateRecord,
) -> Result<(), GuestError> {
    write_game_state_inner(memory, word, state, None)
}

fn write_game_state_inner(
    memory: &mut SyscallMemory,
    word: i32,
    state: &GameStateRecord,
    profile: Option<AbiProfile>,
) -> Result<(), GuestError> {
    if state.string_offsets.len() != 1024 || state.string_data.len() != 16000 {
        return Err(GuestError::invalid("Invalid source gameState_t extent"));
    }
    let range = memory.span(word, QVM_GAME_STATE_BYTES, 0)?;
    for index in 0..1024 {
        let source = match profile {
            None => index as i32,
            Some(profile) => {
                if !profile.is_modern() && (16..=26).contains(&index) {
                    -1
                } else {
                    qvm_configstring(index as i32, profile)?
                }
            }
        };
        let value = if source < 0 {
            0
        } else {
            state.string_offsets.get(source as usize).copied().unwrap_or(0)
        };
        memory.write_i32(range.start + index * 4, value)?;
    }
    memory.write_bytes(range.start + 4096, &state.string_data)?;
    memory.write_i32(range.start + 20096, state.data_count)?;
    Ok(())
}

/// Write a source snapshot record (ping comes from the snapshot's player state).
pub fn write_source_snapshot(
    memory: &mut SyscallMemory,
    word: i32,
    snapshot: &SourceSnapshot,
    profile: AbiProfile,
    source: bool,
) -> Result<(), GuestError> {
    let size = qvm_snapshot_bytes(profile);
    if snapshot.entities.len() > 256 {
        return Err(GuestError::invalid("Invalid source snapshot_t extent"));
    }
    let range = memory.span(word, size, 0)?;
    let ps_bytes = qvm_player_state_bytes(profile);
    let entity_bytes = qvm_entity_state_bytes(profile);
    memory.write_i32(range.start, snapshot.flags)?;
    memory.write_i32(range.start + 4, snapshot.player_state.ping_ms)?;
    memory.write_i32(range.start + 8, snapshot.server_time)?;
    memory.write_bytes(range.start + 12, &snapshot.area_mask)?;
    {
        let mut player = vec![0u8; ps_bytes];
        if source {
            write_source_player_state(&mut player, &snapshot.player_state, profile)?;
        } else {
            write_player_state(&mut player, &snapshot.player_state, profile)?;
        }
        memory.write_bytes(range.start + 44, &player)?;
    }
    memory.write_i32(range.start + 44 + ps_bytes, snapshot.entities.len() as i32)?;
    for (index, entity) in snapshot.entities.iter().enumerate() {
        let mut record = vec![0u8; entity_bytes];
        if source {
            write_source_entity_state(&mut record, entity, profile)?;
        } else {
            write_entity_state(&mut record, entity, profile)?;
        }
        memory.write_bytes(range.start + 48 + ps_bytes + index * entity_bytes, &record)?;
    }
    memory.write_i32(range.start + size - 4, snapshot.server_command_sequence)?;
    Ok(())
}

/// Write a snapshot record with an explicit ping value.
pub fn write_snapshot(
    memory: &mut SyscallMemory,
    word: i32,
    snapshot: &SourceSnapshot,
    ping: i32,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    let mut owned = snapshot.clone();
    owned.player_state.ping_ms = ping;
    write_source_snapshot(memory, word, &owned, profile, false)
}

/// User-command write mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCommandWrite {
    /// Encode a fresh record.
    Encode,
    /// Update in place, preserving legacy button bits 32 and 64.
    Update,
}

/// Write a user command into a 24-byte guest record.
pub fn write_user_command(
    memory: &mut SyscallMemory,
    word: i32,
    command: &WireUserCommand,
    profile: AbiProfile,
    mode: UserCommandWrite,
) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_USER_COMMAND_BYTES, 0)?;
    let layout = qvm_user_command_layout(profile);
    memory.write_i32(range.start, command.server_time)?;
    if layout.buttons_byte {
        let preserved = if mode == UserCommandWrite::Update {
            memory.get(range.start + layout.buttons_offset)? & 96
        } else {
            0
        };
        let buttons = preserved | ((command.buttons & 31) as u8) | (if command.buttons & 2048 != 0 { 128 } else { 0 });
        memory.set(range.start + layout.buttons_offset, buttons)?;
    } else {
        memory.write_i32(range.start + layout.buttons_offset, command.buttons)?;
    }
    for (index, angle) in command.angles.iter().enumerate() {
        memory.write_i32(range.start + layout.angles + index * 4, *angle)?;
    }
    memory.set(range.start + layout.weapon, command.weapon)?;
    memory.write_i8(range.start + layout.forward, command.forwardmove)?;
    memory.write_i8(range.start + layout.right, command.rightmove)?;
    memory.write_i8(range.start + layout.up, command.upmove)?;
    Ok(())
}

/// Read a user command from a 24-byte guest record.
pub fn read_user_command(
    memory: &SyscallMemory,
    word: i32,
    profile: AbiProfile,
) -> Result<WireUserCommand, GuestError> {
    let range = memory.span(word, QVM_USER_COMMAND_BYTES, 0)?;
    let layout = qvm_user_command_layout(profile);
    let buttons = if layout.buttons_byte {
        let raw = memory.get(range.start + layout.buttons_offset)?;
        i32::from(raw & 31) | (if raw & 128 == 0 { 0 } else { 2048 })
    } else {
        memory.read_i32(range.start + layout.buttons_offset)?
    };
    Ok(WireUserCommand {
        server_time: memory.read_i32(range.start)?,
        angles: [
            memory.read_i32(range.start + layout.angles)?,
            memory.read_i32(range.start + layout.angles + 4)?,
            memory.read_i32(range.start + layout.angles + 8)?,
        ],
        buttons,
        weapon: memory.get(range.start + layout.weapon)?,
        forwardmove: memory.read_i8(range.start + layout.forward)?,
        rightmove: memory.read_i8(range.start + layout.right)?,
        upmove: memory.read_i8(range.start + layout.up)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> SyscallMemory {
        SyscallMemory::new(65536).unwrap()
    }

    fn player() -> PlayerStateFields {
        PlayerStateFields {
            command_time_ms: 100,
            movement_type: 1,
            bob_cycle: 2,
            movement_flags: 3,
            movement_time_ms: 4,
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            velocity: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            weapon_time_ms: 7,
            gravity: 800,
            speed: 320,
            delta_angles: [10, 20, 30],
            ground_entity_number: 11,
            legs_timer_ms: 12,
            legs_animation: 13,
            torso_timer_ms: 14,
            torso_animation: 15,
            movement_direction: 16,
            grapple_point: Vec3 { x: 7.0, y: 8.0, z: 9.0 },
            flags: 17,
            event_sequence: 18,
            events: [19, 20],
            event_parameters: [21, 22],
            external_event: 23,
            external_event_parameter: 24,
            external_event_time_ms: 25,
            client_number: 26,
            weapon: 27,
            weapon_state: 28,
            view_angles: Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            view_height: 29,
            damage_event: 30,
            damage_yaw: 31,
            damage_pitch: 32,
            damage_count: 33,
            stats: core::array::from_fn(|index| index as i32),
            persistent: core::array::from_fn(|index| 100 + index as i32),
            powerups: core::array::from_fn(|index| 200 + index as i32),
            ammo: core::array::from_fn(|index| 300 + index as i32),
            generic1: 34,
            loop_sound: 35,
            jump_pad_entity: 36,
            ping_ms: 37,
            movement_frame_count: 38,
            jump_pad_frame: 39,
            entity_event_sequence: 40,
        }
    }

    #[test]
    fn player_round_trip_modern() {
        let mut bytes = vec![0u8; QVM_PLAYER_STATE_BYTES];
        write_player_state(&mut bytes, &player(), AbiProfile::Modern).unwrap();
        assert_eq!(read_player_state(&bytes, AbiProfile::Modern).unwrap(), player());
    }

    #[test]
    fn player_source_round_trip_legacy() {
        let mut bytes = vec![0u8; QVM_LEGACY_PLAYER_STATE_BYTES];
        let mut state = player();
        state.powerups = [0i32; 16];
        write_source_player_state(&mut bytes, &state, AbiProfile::Legacy).unwrap();
        let back = read_source_player_state(&bytes, AbiProfile::Legacy).unwrap();
        assert_eq!(back.command_time_ms, 100);
        assert_eq!(back.ping_ms, 37);
        assert_eq!(back.generic1, 0);
    }

    #[test]
    fn player_write_rejects_legacy_powerups() {
        let mut bytes = vec![0u8; QVM_LEGACY_PLAYER_STATE_BYTES];
        assert!(write_player_state(&mut bytes, &player(), AbiProfile::Legacy).is_err());
    }

    #[test]
    fn entity_round_trip_modern() {
        let state = EntityStateFields {
            number: 5,
            entity_type: 3,
            event: 44,
            generic1: 9,
            ..EntityStateFields::default()
        };
        let mut bytes = vec![0u8; QVM_ENTITY_STATE_BYTES];
        write_entity_state(&mut bytes, &state, AbiProfile::Modern).unwrap();
        assert_eq!(read_entity_state(&bytes, AbiProfile::Modern).unwrap(), state);
    }

    #[test]
    fn entity_write_rejects_legacy_generic1() {
        let state = EntityStateFields {
            generic1: 1,
            ..EntityStateFields::default()
        };
        let mut bytes = vec![0u8; QVM_LEGACY_ENTITY_STATE_BYTES];
        assert!(write_entity_state(&mut bytes, &state, AbiProfile::Legacy).is_err());
    }

    #[test]
    fn game_state_writes_offsets_and_data() {
        let mut memory = memory();
        let state = GameStateRecord {
            string_offsets: (0..1024).map(|index| index * 4).collect(),
            string_data: vec![0x55u8; 16000],
            data_count: 16000,
        };
        write_game_state(&mut memory, 4096, &state, AbiProfile::Modern).unwrap();
        assert_eq!(memory.read_i32(4096).unwrap(), 0);
        assert_eq!(memory.read_i32(4096 + 4).unwrap(), 4);
        assert_eq!(memory.get(4096 + 4096).unwrap(), 0x55);
        assert_eq!(memory.read_i32(4096 + 20096).unwrap(), 16000);
    }

    #[test]
    fn game_state_rejects_bad_extent() {
        let mut memory = memory();
        let state = GameStateRecord {
            string_offsets: vec![0; 10],
            string_data: vec![0; 16000],
            data_count: 0,
        };
        assert!(write_game_state(&mut memory, 4096, &state, AbiProfile::Modern).is_err());
    }

    #[test]
    fn snapshot_writes_header_and_tail() {
        let mut memory = memory();
        let mut state = player();
        state.powerups = [0i32; 16];
        let snapshot = SourceSnapshot {
            number: 7,
            server_time: 4242,
            flags: 3,
            area_mask: [0xABu8; 32],
            player_state: state,
            entities: vec![EntityStateFields::default(), EntityStateFields::default()],
            server_command_sequence: 99,
        };
        write_snapshot(&mut memory, 1024, &snapshot, 37, AbiProfile::Modern).unwrap();
        assert_eq!(memory.read_i32(1024).unwrap(), 3);
        assert_eq!(memory.read_i32(1024 + 4).unwrap(), 37);
        assert_eq!(memory.read_i32(1024 + 8).unwrap(), 4242);
        assert_eq!(memory.get(1024 + 12).unwrap(), 0xAB);
        assert_eq!(memory.read_i32(1024 + 44 + 468).unwrap(), 2);
        assert_eq!(memory.read_i32(1024 + QVM_SNAPSHOT_BYTES - 4).unwrap(), 99);
    }

    #[test]
    fn snapshot_rejects_too_many_entities() {
        let mut memory = memory();
        let snapshot = SourceSnapshot {
            number: 0,
            server_time: 0,
            flags: 0,
            area_mask: [0u8; 32],
            player_state: player(),
            entities: vec![EntityStateFields::default(); 257],
            server_command_sequence: 0,
        };
        assert!(write_snapshot(&mut memory, 1024, &snapshot, 0, AbiProfile::Modern).is_err());
    }

    #[test]
    fn user_command_round_trip_modern() {
        let mut memory = memory();
        let command = WireUserCommand {
            server_time: 123,
            angles: [1, 2, 3],
            buttons: 0x1F2F,
            weapon: 7,
            forwardmove: 100,
            rightmove: -50,
            upmove: 10,
        };
        write_user_command(&mut memory, 512, &command, AbiProfile::Modern, UserCommandWrite::Encode).unwrap();
        assert_eq!(read_user_command(&memory, 512, AbiProfile::Modern).unwrap(), command);
    }

    #[test]
    fn user_command_round_trip_legacy() {
        let mut memory = memory();
        let command = WireUserCommand {
            server_time: 321,
            angles: [4, 5, 6],
            buttons: 31 | 2048,
            weapon: 3,
            forwardmove: -100,
            rightmove: 50,
            upmove: -10,
        };
        write_user_command(&mut memory, 512, &command, AbiProfile::Legacy, UserCommandWrite::Encode).unwrap();
        assert_eq!(memory.get(512 + 4).unwrap(), 31 | 128);
        assert_eq!(read_user_command(&memory, 512, AbiProfile::Legacy).unwrap(), command);
    }

    #[test]
    fn user_command_update_preserves_legacy_bits() {
        let mut memory = memory();
        memory.set(512 + 4, 96).unwrap();
        let command = WireUserCommand {
            buttons: 5,
            ..WireUserCommand::default()
        };
        write_user_command(&mut memory, 512, &command, AbiProfile::Legacy, UserCommandWrite::Update).unwrap();
        assert_eq!(memory.get(512 + 4).unwrap(), 96 | 5);
    }
}
