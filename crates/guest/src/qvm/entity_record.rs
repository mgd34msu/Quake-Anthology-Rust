//! QVM `entityState_t` record codec with legacy ABI translation.
//!
//! Provenance: `src/compat/qvm/entity-record.ts` (port of id Software's
//! `code/game/q_shared.h` `trajectory_t`/`entityState_t`).
//!
//! Local mirrors of `src/compat/qvm/legacy-presentation.ts`
//! ([`qvm_event`], [`qvm_entity_type`], [`qvm_persistent`],
//! [`qvm_configstring`], [`qvm_powerup_bits`], [`qvm_powerups`]); the sibling
//! `legacy_presentation` module owns the canonical port with identical
//! signatures, so the parent can unify these by import path.
//!
//! The donor borrows live fields through a `DataView`; this port uses owned
//! [`QvmEntityState`] values with explicit read/write functions over byte
//! slices, which keeps guest parsing total and side-effect free.

use qa_core::math::{vec3, Vec3};

use super::game_data::AbiProfile;
use crate::error::GuestError;

/// Modern `entityState_t` size in bytes (legacy records are 204 bytes).
pub const QVM_ENTITY_STATE_BYTES: usize = 208;

/// Record size for one ABI profile.
#[must_use]
pub fn qvm_entity_state_bytes(profile: AbiProfile) -> usize {
    if profile.is_modern() {
        QVM_ENTITY_STATE_BYTES
    } else {
        204
    }
}

/// Legacy event translation table (donor `legacy-presentation.ts`).
const LEGACY_EVENTS: &[i32] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46,
    48, 49, 50, 51, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 66, 68,
];

/// Translate an event number between the legacy and modern ABIs.
pub fn qvm_event(value: i32, profile: AbiProfile, reverse: bool) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(value);
    }
    let event = value & 255;
    let flags = value & !255;
    let mapped = if reverse {
        LEGACY_EVENTS.iter().position(|known| *known == event).map(|index| index as i32)
    } else {
        LEGACY_EVENTS.get(event as usize).copied()
    };
    match mapped {
        Some(mapped) if mapped >= 0 => Ok(mapped | flags),
        _ => Err(GuestError::invalid(format!(
            "QVM event {event} is not represented by the selected legacy ABI"
        ))),
    }
}

/// Translate an entity type between the legacy and modern ABIs.
pub fn qvm_entity_type(
    value: i32,
    profile: AbiProfile,
    reverse: bool,
) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(value);
    }
    let source_events = if reverse { 13 } else { 12 };
    let target_events = if reverse { 12 } else { 13 };
    if value >= source_events {
        return Ok(target_events + qvm_event(value - source_events, profile, reverse)?);
    }
    if reverse && value == 12 {
        return Err(GuestError::invalid("Legacy QVM has no team entity type"));
    }
    Ok(value)
}

/// Translate persistent slots to the modern presentation (legacy only).
#[must_use]
pub fn qvm_persistent(values: &[i32; 16], profile: AbiProfile) -> [i32; 16] {
    if profile.is_modern() {
        return *values;
    }
    let mut result = [0i32; 16];
    result[0] = values[0];
    result[1] = values[1];
    result[2] = values[2];
    result[3] = values[3];
    result[4] = values[4];
    result[6] = values[7];
    result[8] = values[8];
    result[9] = values[9];
    result[10] = values[10];
    result[13] = values[11];
    result
}

/// Translate a configstring index to the modern ABI.
pub fn qvm_configstring(index: i32, profile: AbiProfile) -> Result<i32, GuestError> {
    if profile.is_modern() {
        return Ok(index);
    }
    if (12..=15).contains(&index) {
        return Ok(index + 8);
    }
    if (16..=26).contains(&index) {
        return Err(GuestError::invalid(format!(
            "Legacy private configstring {index} has no declared modern presentation mapping"
        )));
    }
    Ok(index)
}

/// Validate powerup bits against the modern presentation mapping.
pub fn qvm_powerup_bits(bits: i32, profile: AbiProfile) -> Result<i32, GuestError> {
    if !profile.is_modern() && (bits & !0x1ff) != 0 {
        return Err(GuestError::invalid(
            "Legacy ball or private powerup has no modern presentation mapping",
        ));
    }
    Ok(bits)
}

/// Validate powerup slots against the modern presentation mapping.
pub fn qvm_powerups(values: &[i32; 16], profile: AbiProfile) -> Result<[i32; 16], GuestError> {
    if !profile.is_modern() && values[9..].iter().any(|value| *value != 0) {
        return Err(GuestError::invalid(
            "Legacy ball or private powerup has no modern presentation mapping",
        ));
    }
    Ok(*values)
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

/// Shared trajectory record (`trajectory_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmTrajectory {
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

impl Default for QvmTrajectory {
    fn default() -> Self {
        Self {
            trajectory_type: 0,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

fn read_trajectory(bytes: &[u8], offset: usize) -> QvmTrajectory {
    QvmTrajectory {
        trajectory_type: read_i32(bytes, offset),
        time: read_i32(bytes, offset + 4),
        duration: read_i32(bytes, offset + 8),
        base: read_vec3(bytes, offset + 12),
        delta: read_vec3(bytes, offset + 24),
    }
}

fn write_trajectory(bytes: &mut [u8], offset: usize, value: &QvmTrajectory) {
    write_i32(bytes, offset, value.trajectory_type);
    write_i32(bytes, offset + 4, value.time);
    write_i32(bytes, offset + 8, value.duration);
    write_vec3(bytes, offset + 12, &value.base);
    write_vec3(bytes, offset + 24, &value.delta);
}

/// Owned `entityState_t` record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmEntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: QvmTrajectory,
    /// Angular trajectory.
    pub apos: QvmTrajectory,
    /// Time.
    pub time: i32,
    /// Secondary time.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
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
    /// Powerup bits.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic field (modern only; always 0 on legacy).
    pub generic1: i32,
}

impl Default for QvmEntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: QvmTrajectory::default(),
            apos: QvmTrajectory::default(),
            time: 0,
            time2: 0,
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 0,
            other_entity_num2: 0,
            ground_entity_num: 0,
            constant_light: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            solid: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: 0,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }
}

fn check_record(bytes: &[u8], profile: AbiProfile) -> Result<(), GuestError> {
    let need = qvm_entity_state_bytes(profile);
    if bytes.len() < need {
        return Err(GuestError::invalid(format!(
            "QVM entityState_t record requires {need} bytes, received {}",
            bytes.len()
        )));
    }
    Ok(())
}

/// Read a translated entity state: enum tags convert to modern values.
pub fn read_qvm_entity_state(bytes: &[u8], profile: AbiProfile) -> Result<QvmEntityState, GuestError> {
    let mut state = read_source_qvm_entity_state(bytes, profile)?;
    state.e_type = qvm_entity_type(state.e_type, profile, false)?;
    state.event = qvm_event(state.event, profile, false)?;
    state.powerups = qvm_powerup_bits(state.powerups, profile)?;
    Ok(state)
}

/// Read source entity state without presentation translation.
pub fn read_source_qvm_entity_state(
    bytes: &[u8],
    profile: AbiProfile,
) -> Result<QvmEntityState, GuestError> {
    check_record(bytes, profile)?;
    Ok(QvmEntityState {
        number: read_i32(bytes, 0),
        e_type: read_i32(bytes, 4),
        e_flags: read_i32(bytes, 8),
        pos: read_trajectory(bytes, 12),
        apos: read_trajectory(bytes, 48),
        time: read_i32(bytes, 84),
        time2: read_i32(bytes, 88),
        origin: read_vec3(bytes, 92),
        origin2: read_vec3(bytes, 104),
        angles: read_vec3(bytes, 116),
        angles2: read_vec3(bytes, 128),
        other_entity_num: read_i32(bytes, 140),
        other_entity_num2: read_i32(bytes, 144),
        ground_entity_num: read_i32(bytes, 148),
        constant_light: read_i32(bytes, 152),
        loop_sound: read_i32(bytes, 156),
        modelindex: read_i32(bytes, 160),
        modelindex2: read_i32(bytes, 164),
        client_num: read_i32(bytes, 168),
        frame: read_i32(bytes, 172),
        solid: read_i32(bytes, 176),
        event: read_i32(bytes, 180),
        event_parm: read_i32(bytes, 184),
        powerups: read_i32(bytes, 188),
        weapon: read_i32(bytes, 192),
        legs_anim: read_i32(bytes, 196),
        torso_anim: read_i32(bytes, 200),
        generic1: if profile.is_modern() {
            read_i32(bytes, 204)
        } else {
            0
        },
    })
}

/// Write a translated entity state: modern tags convert to source values.
pub fn write_qvm_entity_state(
    bytes: &mut [u8],
    state: &QvmEntityState,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    let translated = QvmEntityState {
        e_type: qvm_entity_type(state.e_type, profile, true)?,
        event: qvm_event(state.event, profile, true)?,
        powerups: qvm_powerup_bits(state.powerups, profile)?,
        ..state.clone()
    };
    write_source_qvm_entity_state(bytes, &translated, profile)
}

/// Write source entity state without presentation translation.
pub fn write_source_qvm_entity_state(
    bytes: &mut [u8],
    state: &QvmEntityState,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    check_record(bytes, profile)?;
    write_i32(bytes, 0, state.number);
    write_i32(bytes, 4, state.e_type);
    write_i32(bytes, 8, state.e_flags);
    write_trajectory(bytes, 12, &state.pos);
    write_trajectory(bytes, 48, &state.apos);
    write_i32(bytes, 84, state.time);
    write_i32(bytes, 88, state.time2);
    write_vec3(bytes, 92, &state.origin);
    write_vec3(bytes, 104, &state.origin2);
    write_vec3(bytes, 116, &state.angles);
    write_vec3(bytes, 128, &state.angles2);
    write_i32(bytes, 140, state.other_entity_num);
    write_i32(bytes, 144, state.other_entity_num2);
    write_i32(bytes, 148, state.ground_entity_num);
    write_i32(bytes, 152, state.constant_light);
    write_i32(bytes, 156, state.loop_sound);
    write_i32(bytes, 160, state.modelindex);
    write_i32(bytes, 164, state.modelindex2);
    write_i32(bytes, 168, state.client_num);
    write_i32(bytes, 172, state.frame);
    write_i32(bytes, 176, state.solid);
    write_i32(bytes, 180, state.event);
    write_i32(bytes, 184, state.event_parm);
    write_i32(bytes, 188, state.powerups);
    write_i32(bytes, 192, state.weapon);
    write_i32(bytes, 196, state.legs_anim);
    write_i32(bytes, 200, state.torso_anim);
    if profile.is_modern() {
        write_i32(bytes, 204, state.generic1);
    } else if state.generic1 != 0 {
        return Err(GuestError::invalid("Legacy QVM entity has no generic1 field"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QvmEntityState {
        QvmEntityState {
            number: 7,
            e_type: 3,
            e_flags: 9,
            time: 100,
            time2: 200,
            origin: vec3(1.0, 2.0, 3.0),
            origin2: vec3(4.0, 5.0, 6.0),
            angles: vec3(7.0, 8.0, 9.0),
            angles2: vec3(10.0, 11.0, 12.0),
            other_entity_num: 13,
            other_entity_num2: 14,
            ground_entity_num: 15,
            constant_light: 16,
            loop_sound: 17,
            modelindex: 18,
            modelindex2: 19,
            client_num: 20,
            frame: 21,
            solid: 22,
            event: 23,
            event_parm: 24,
            powerups: 0x1f,
            weapon: 26,
            legs_anim: 27,
            torso_anim: 28,
            generic1: 29,
            pos: QvmTrajectory {
                trajectory_type: 1,
                time: 30,
                duration: 31,
                base: vec3(32.0, 33.0, 34.0),
                delta: vec3(35.0, 36.0, 37.0),
            },
            apos: QvmTrajectory {
                trajectory_type: 2,
                time: 38,
                duration: 39,
                base: vec3(40.0, 41.0, 42.0),
                delta: vec3(43.0, 44.0, 45.0),
            },
            ..QvmEntityState::default()
        }
    }

    #[test]
    fn modern_round_trip_preserves_every_field() {
        let state = sample();
        let mut bytes = vec![0u8; QVM_ENTITY_STATE_BYTES];
        write_qvm_entity_state(&mut bytes, &state, AbiProfile::Modern).unwrap();
        assert_eq!(read_qvm_entity_state(&bytes, AbiProfile::Modern).unwrap(), state);
        assert_eq!(i32::from_le_bytes(bytes[0..4].try_into().unwrap()), 7);
        assert_eq!(i32::from_le_bytes(bytes[204..208].try_into().unwrap()), 29);
    }

    #[test]
    fn legacy_generic1_is_forced_zero() {
        let mut state = sample();
        state.generic1 = 0;
        let mut bytes = vec![0u8; 204];
        write_qvm_entity_state(&mut bytes, &state, AbiProfile::Legacy).unwrap();
        let back = read_qvm_entity_state(&bytes, AbiProfile::Legacy).unwrap();
        assert_eq!(back.generic1, 0);
        assert_eq!(back.number, 7);
        state.generic1 = 1;
        assert!(write_qvm_entity_state(&mut bytes, &state, AbiProfile::Legacy).is_err());
    }

    #[test]
    fn legacy_event_translation_round_trips() {
        assert_eq!(qvm_event(47, AbiProfile::Legacy, false).unwrap(), 48);
        assert_eq!(qvm_event(48, AbiProfile::Legacy, true).unwrap(), 47);
        assert_eq!(qvm_event(0x101, AbiProfile::Legacy, false).unwrap(), 0x101);
        assert!(qvm_event(200, AbiProfile::Legacy, false).is_err());
        assert_eq!(qvm_entity_type(12, AbiProfile::Legacy, false).unwrap(), 13);
        assert!(qvm_entity_type(12, AbiProfile::Legacy, true).is_err());
    }

    #[test]
    fn legacy_slot_validation_matches_donor() {
        assert!(qvm_powerup_bits(0x200, AbiProfile::Legacy).is_err());
        assert_eq!(qvm_powerup_bits(0x1ff, AbiProfile::Legacy).unwrap(), 0x1ff);
        let mut slots = [0i32; 16];
        slots[9] = 1;
        assert!(qvm_powerups(&slots, AbiProfile::Legacy).is_err());
        slots[9] = 0;
        assert!(qvm_powerups(&slots, AbiProfile::Legacy).is_ok());
        let full: [i32; 16] = core::array::from_fn(|index| index as i32);
        let mapped = qvm_persistent(&full, AbiProfile::Legacy);
        assert_eq!((mapped[6], mapped[13], mapped[7], mapped[11]), (7, 11, 0, 0));
        assert_eq!(qvm_configstring(12, AbiProfile::Legacy).unwrap(), 20);
        assert!(qvm_configstring(16, AbiProfile::Legacy).is_err());
    }

    #[test]
    fn short_records_rejected_before_mutation() {
        let short = vec![0u8; 100];
        assert!(read_qvm_entity_state(&short, AbiProfile::Modern).is_err());
        let mut short = vec![0u8; 100];
        assert!(write_qvm_entity_state(&mut short, &sample(), AbiProfile::Modern).is_err());
        assert_eq!(short, vec![0u8; 100]);
    }
}
