//! QVM `playerState_t` record codec.
//!
//! Provenance: `src/compat/qvm/player-record.ts` (port of id Software's
//! `code/game/q_shared.h` `playerState_t`).
//!
//! [`QvmPlayerState`] mirrors `src/contracts/protocol.ts` `Q3PlayerState`
//! (owned by another worker); legacy enum translation reuses the
//! [`super::entity_record`] mirrors of `legacy-presentation.ts`.

use qa_core::math::{vec3, Vec3};

use super::entity_record::{qvm_event, qvm_persistent, qvm_powerups};
use super::game_data::AbiProfile;
use crate::error::GuestError;

/// Modern `playerState_t` size in bytes (legacy records are 444 bytes).
pub const QVM_PLAYER_STATE_BYTES: usize = 468;

/// Record size for one ABI profile.
#[must_use]
pub fn qvm_player_state_bytes(profile: AbiProfile) -> usize {
    if profile.is_modern() {
        QVM_PLAYER_STATE_BYTES
    } else {
        444
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    i32::from_le_bytes(word)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    f32::from_le_bytes(word)
}

fn read_vec3(bytes: &[u8], offset: usize) -> Vec3 {
    vec3(
        read_f32(bytes, offset),
        read_f32(bytes, offset + 4),
        read_f32(bytes, offset + 8),
    )
}

fn read_slots(bytes: &[u8], offset: usize) -> [i32; 16] {
    core::array::from_fn(|index| read_i32(bytes, offset + index * 4))
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_vec3(bytes: &mut [u8], offset: usize, value: &Vec3) {
    write_f32(bytes, offset, value.x);
    write_f32(bytes, offset + 4, value.y);
    write_f32(bytes, offset + 8, value.z);
}

fn write_slots(bytes: &mut [u8], offset: usize, slots: &[i32; 16]) {
    for (index, value) in slots.iter().enumerate() {
        write_i32(bytes, offset + index * 4, *value);
    }
}

/// Owned Quake III player state (mirror of `Q3PlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPlayerState {
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
    pub delta_angle_words: [i32; 3],
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
    /// Predictable events.
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
    /// Generic field (modern only).
    pub generic1: i32,
    /// Loop sound (modern only).
    pub loop_sound: i32,
    /// Jump-pad entity (modern only).
    pub jump_pad_entity: i32,
    /// Ping in milliseconds (host-copied, outside network delta).
    pub ping_ms: i32,
    /// Movement frame count (modern only).
    pub movement_frame_count: i32,
    /// Jump-pad frame (modern only).
    pub jump_pad_frame: i32,
    /// Entity event sequence (modern only).
    pub entity_event_sequence: i32,
}

impl Default for QvmPlayerState {
    fn default() -> Self {
        Self {
            command_time_ms: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_ms: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time_ms: 0,
            gravity: 0,
            speed: 0,
            delta_angle_words: [0; 3],
            ground_entity_number: 0,
            legs_timer_ms: 0,
            legs_animation: 0,
            torso_timer_ms: 0,
            torso_animation: 0,
            movement_direction: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            flags: 0,
            event_sequence: 0,
            events: [0; 2],
            event_parameters: [0; 2],
            external_event: 0,
            external_event_parameter: 0,
            external_event_time_ms: 0,
            client_number: 0,
            weapon: 0,
            weapon_state: 0,
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: [0; 16],
            persistent: [0; 16],
            powerups: [0; 16],
            ammo: [0; 16],
            generic1: 0,
            loop_sound: 0,
            jump_pad_entity: 0,
            ping_ms: 0,
            movement_frame_count: 0,
            jump_pad_frame: 0,
            entity_event_sequence: 0,
        }
    }
}

fn check_record(bytes: &[u8], profile: AbiProfile) -> Result<(), GuestError> {
    let need = qvm_player_state_bytes(profile);
    if bytes.len() < need {
        return Err(GuestError::invalid(format!(
            "QVM playerState_t requires {need} bytes, got {}",
            bytes.len()
        )));
    }
    Ok(())
}

/// Read a translated player state: enum tags convert to modern values.
pub fn read_qvm_player_state(bytes: &[u8], profile: AbiProfile) -> Result<QvmPlayerState, GuestError> {
    let source = read_source_qvm_player_state(bytes, profile)?;
    Ok(QvmPlayerState {
        events: [
            qvm_event(source.events[0], profile, false)?,
            qvm_event(source.events[1], profile, false)?,
        ],
        external_event: qvm_event(source.external_event, profile, false)?,
        persistent: qvm_persistent(&source.persistent, profile),
        powerups: qvm_powerups(&source.powerups, profile)?,
        ..source
    })
}

/// Read source player state without presentation translation.
pub fn read_source_qvm_player_state(bytes: &[u8], profile: AbiProfile) -> Result<QvmPlayerState, GuestError> {
    check_record(bytes, profile)?;
    let modern = profile.is_modern();
    Ok(QvmPlayerState {
        command_time_ms: read_i32(bytes, 0),
        movement_type: read_i32(bytes, 4),
        bob_cycle: read_i32(bytes, 8),
        movement_flags: read_i32(bytes, 12),
        movement_time_ms: read_i32(bytes, 16),
        origin: read_vec3(bytes, 20),
        velocity: read_vec3(bytes, 32),
        weapon_time_ms: read_i32(bytes, 44),
        gravity: read_i32(bytes, 48),
        speed: read_i32(bytes, 52),
        delta_angle_words: [read_i32(bytes, 56), read_i32(bytes, 60), read_i32(bytes, 64)],
        ground_entity_number: read_i32(bytes, 68),
        legs_timer_ms: read_i32(bytes, 72),
        legs_animation: read_i32(bytes, 76),
        torso_timer_ms: read_i32(bytes, 80),
        torso_animation: read_i32(bytes, 84),
        movement_direction: read_i32(bytes, 88),
        grapple_point: read_vec3(bytes, 92),
        flags: read_i32(bytes, 104),
        event_sequence: read_i32(bytes, 108),
        events: [read_i32(bytes, 112), read_i32(bytes, 116)],
        event_parameters: [read_i32(bytes, 120), read_i32(bytes, 124)],
        external_event: read_i32(bytes, 128),
        external_event_parameter: read_i32(bytes, 132),
        external_event_time_ms: read_i32(bytes, 136),
        client_number: read_i32(bytes, 140),
        weapon: read_i32(bytes, 144),
        weapon_state: read_i32(bytes, 148),
        view_angles: read_vec3(bytes, 152),
        view_height: read_i32(bytes, 164),
        damage_event: read_i32(bytes, 168),
        damage_yaw: read_i32(bytes, 172),
        damage_pitch: read_i32(bytes, 176),
        damage_count: read_i32(bytes, 180),
        stats: read_slots(bytes, 184),
        persistent: read_slots(bytes, 248),
        powerups: read_slots(bytes, 312),
        ammo: read_slots(bytes, 376),
        generic1: if modern { read_i32(bytes, 440) } else { 0 },
        loop_sound: if modern { read_i32(bytes, 444) } else { 0 },
        jump_pad_entity: if modern { read_i32(bytes, 448) } else { 0 },
        ping_ms: read_i32(bytes, if modern { 452 } else { 440 }),
        movement_frame_count: if modern { read_i32(bytes, 456) } else { 0 },
        jump_pad_frame: if modern { read_i32(bytes, 460) } else { 0 },
        entity_event_sequence: if modern { read_i32(bytes, 464) } else { 0 },
    })
}

/// Legacy persistent projection for translated writes.
fn legacy_persistent(state: &QvmPlayerState, bytes: &[u8], preserve_private: bool) -> [i32; 16] {
    let mut persistent = if preserve_private {
        read_slots(bytes, 248)
    } else {
        [0; 16]
    };
    for index in [0, 1, 2, 3, 4, 8, 9, 10] {
        persistent[index] = state.persistent[index];
    }
    persistent[7] = state.persistent[6];
    persistent[11] = state.persistent[13];
    persistent
}

/// Write a translated player state, initializing legacy private slots.
pub fn write_qvm_player_state(bytes: &mut [u8], state: &QvmPlayerState, profile: AbiProfile) -> Result<(), GuestError> {
    write_qvm_player_state_inner(bytes, state, profile, false)
}

/// Write a translated player state, preserving legacy private slots.
pub fn write_qvm_player_state_preserve(
    bytes: &mut [u8],
    state: &QvmPlayerState,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    write_qvm_player_state_inner(bytes, state, profile, true)
}

fn write_qvm_player_state_inner(
    bytes: &mut [u8],
    state: &QvmPlayerState,
    profile: AbiProfile,
    preserve_private: bool,
) -> Result<(), GuestError> {
    check_record(bytes, profile)?;
    let persistent = if profile.is_modern() {
        state.persistent
    } else {
        legacy_persistent(state, bytes, preserve_private)
    };
    let translated = QvmPlayerState {
        persistent,
        events: [
            qvm_event(state.events[0], profile, true)?,
            qvm_event(state.events[1], profile, true)?,
        ],
        external_event: qvm_event(state.external_event, profile, true)?,
        powerups: qvm_powerups(&state.powerups, profile)?,
        ..state.clone()
    };
    write_source_qvm_player_state(bytes, &translated, profile)
}

/// Write source player state without presentation translation.
pub fn write_source_qvm_player_state(
    bytes: &mut [u8],
    state: &QvmPlayerState,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    check_record(bytes, profile)?;
    write_i32(bytes, 0, state.command_time_ms);
    write_i32(bytes, 4, state.movement_type);
    write_i32(bytes, 8, state.bob_cycle);
    write_i32(bytes, 12, state.movement_flags);
    write_i32(bytes, 16, state.movement_time_ms);
    write_vec3(bytes, 20, &state.origin);
    write_vec3(bytes, 32, &state.velocity);
    write_i32(bytes, 44, state.weapon_time_ms);
    write_i32(bytes, 48, state.gravity);
    write_i32(bytes, 52, state.speed);
    write_i32(bytes, 56, state.delta_angle_words[0]);
    write_i32(bytes, 60, state.delta_angle_words[1]);
    write_i32(bytes, 64, state.delta_angle_words[2]);
    write_i32(bytes, 68, state.ground_entity_number);
    write_i32(bytes, 72, state.legs_timer_ms);
    write_i32(bytes, 76, state.legs_animation);
    write_i32(bytes, 80, state.torso_timer_ms);
    write_i32(bytes, 84, state.torso_animation);
    write_i32(bytes, 88, state.movement_direction);
    write_vec3(bytes, 92, &state.grapple_point);
    write_i32(bytes, 104, state.flags);
    write_i32(bytes, 108, state.event_sequence);
    write_i32(bytes, 112, state.events[0]);
    write_i32(bytes, 116, state.events[1]);
    write_i32(bytes, 120, state.event_parameters[0]);
    write_i32(bytes, 124, state.event_parameters[1]);
    write_i32(bytes, 128, state.external_event);
    write_i32(bytes, 132, state.external_event_parameter);
    write_i32(bytes, 136, state.external_event_time_ms);
    write_i32(bytes, 140, state.client_number);
    write_i32(bytes, 144, state.weapon);
    write_i32(bytes, 148, state.weapon_state);
    write_vec3(bytes, 152, &state.view_angles);
    write_i32(bytes, 164, state.view_height);
    write_i32(bytes, 168, state.damage_event);
    write_i32(bytes, 172, state.damage_yaw);
    write_i32(bytes, 176, state.damage_pitch);
    write_i32(bytes, 180, state.damage_count);
    write_slots(bytes, 184, &state.stats);
    write_slots(bytes, 248, &state.persistent);
    write_slots(bytes, 312, &state.powerups);
    write_slots(bytes, 376, &state.ammo);
    if !profile.is_modern() {
        write_i32(bytes, 440, state.ping_ms);
        return Ok(());
    }
    write_i32(bytes, 440, state.generic1);
    write_i32(bytes, 444, state.loop_sound);
    write_i32(bytes, 448, state.jump_pad_entity);
    write_i32(bytes, 452, state.ping_ms);
    write_i32(bytes, 456, state.movement_frame_count);
    write_i32(bytes, 460, state.jump_pad_frame);
    write_i32(bytes, 464, state.entity_event_sequence);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QvmPlayerState {
        QvmPlayerState {
            command_time_ms: 1000,
            movement_type: 2,
            origin: vec3(1.0, 2.0, 3.0),
            velocity: vec3(4.0, 5.0, 6.0),
            delta_angle_words: [7, 8, 9],
            event_sequence: 11,
            events: [19, 23],
            event_parameters: [1, 2],
            external_event: 45,
            client_number: 3,
            weapon: 5,
            view_angles: vec3(10.0, 20.0, 30.0),
            view_height: 26,
            stats: core::array::from_fn(|index| index as i32),
            persistent: core::array::from_fn(|index| 100 + index as i32),
            powerups: core::array::from_fn(|index| 200 + index as i32),
            ammo: core::array::from_fn(|index| 300 + index as i32),
            generic1: 31,
            ping_ms: 87,
            movement_frame_count: 41,
            ..QvmPlayerState::default()
        }
    }

    #[test]
    fn modern_round_trip_preserves_every_field() {
        let state = sample();
        let mut bytes = vec![0u8; QVM_PLAYER_STATE_BYTES];
        write_qvm_player_state(&mut bytes, &state, AbiProfile::Modern).unwrap();
        assert_eq!(read_qvm_player_state(&bytes, AbiProfile::Modern).unwrap(), state);
        assert_eq!(i32::from_le_bytes(bytes[452..456].try_into().unwrap()), 87);
    }

    #[test]
    fn legacy_tail_fields_read_zero() {
        let mut state = sample();
        state.powerups = [0; 16];
        let mut bytes = vec![0u8; 444];
        write_qvm_player_state(&mut bytes, &state, AbiProfile::Legacy).unwrap();
        let back = read_qvm_player_state(&bytes, AbiProfile::Legacy).unwrap();
        assert_eq!(back.generic1, 0);
        assert_eq!(back.movement_frame_count, 0);
        assert_eq!(back.ping_ms, 87);
        assert_eq!(back.weapon, 5);
    }

    #[test]
    fn legacy_private_slots_preserved_on_request() {
        let mut bytes = vec![0u8; 444];
        for (index, slot) in [0i32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
            .iter()
            .enumerate()
        {
            bytes[248 + index * 4..252 + index * 4].copy_from_slice(&slot.to_le_bytes());
        }
        let mut state = sample();
        state.powerups = [0; 16];
        write_qvm_player_state_preserve(&mut bytes, &state, AbiProfile::Legacy).unwrap();
        let raw = read_slots(&bytes, 248);
        assert_eq!(raw[5], 5);
        assert_eq!(raw[0], 100);
        assert_eq!(raw[7], 106);
        assert_eq!(raw[11], 113);
        write_qvm_player_state(&mut bytes, &state, AbiProfile::Legacy).unwrap();
        let raw = read_slots(&bytes, 248);
        assert_eq!(raw[5], 0);
        assert_eq!(raw[0], 100);
    }

    #[test]
    fn short_records_rejected_before_mutation() {
        let short = vec![0u8; 100];
        assert!(read_qvm_player_state(&short, AbiProfile::Modern).is_err());
        let mut short = vec![0u8; 100];
        assert!(write_qvm_player_state(&mut short, &sample(), AbiProfile::Modern).is_err());
        assert_eq!(short, vec![0u8; 100]);
    }
}
