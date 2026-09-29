//! QVM `sharedEntity_t` record codec.
//!
//! Provenance: `src/compat/qvm/shared-entity-record.ts` (port of id
//! Software's `code/game/g_public.h` `sharedEntity_t` and
//! `code/server/sv_world.c` `SV_ClipHandleForEntity`).
//!
//! The donor borrows the source prefix through a `DataView`; this port uses
//! owned [`QvmSharedEntity`] values with explicit read/write functions. The
//! unused `r.s` gap (bytes 208..415) is preserved untouched by writes.

use qa_core::math::{vec3, Vec3};

use super::entity_record::{read_source_qvm_entity_state, write_source_qvm_entity_state, QvmEntityState};
use super::game_data::AbiProfile;
use crate::error::GuestError;

/// Modern `sharedEntity_t` size in bytes (legacy records are 504 bytes).
pub const QVM_SHARED_ENTITY_BYTES: usize = 516;

/// Record size for one ABI profile.
#[must_use]
pub fn qvm_shared_entity_bytes(profile: AbiProfile) -> usize {
    if profile.is_modern() {
        QVM_SHARED_ENTITY_BYTES
    } else {
        504
    }
}

/// Server flag selecting capsule collision.
const SVF_CAPSULE: i32 = 0x0000_0400;

/// Modern `r` field offsets.
const LINKED: usize = 416;
const LINKCOUNT: usize = 420;
const SV_FLAGS: usize = 424;
const SINGLE_CLIENT: usize = 428;
const BMODEL: usize = 432;
const MINS: usize = 436;
const MAXS: usize = 448;
const CONTENTS: usize = 460;
const ABSMIN: usize = 464;
const ABSMAX: usize = 476;
const CURRENT_ORIGIN: usize = 488;
const CURRENT_ANGLES: usize = 500;
const OWNER_NUM: usize = 512;

/// Map a modern `r` offset to the legacy layout.
fn legacy_offset(modern: usize) -> usize {
    if modern < 428 {
        modern - 8
    } else {
        modern - 12
    }
}

fn offset(profile: AbiProfile, modern: usize) -> usize {
    if profile.is_modern() {
        modern
    } else {
        legacy_offset(modern)
    }
}

fn read_i32(bytes: &[u8], at: usize) -> i32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    i32::from_le_bytes(word)
}

fn read_f32(bytes: &[u8], at: usize) -> f32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    f32::from_le_bytes(word)
}

fn read_vec3(bytes: &[u8], at: usize) -> Vec3 {
    vec3(read_f32(bytes, at), read_f32(bytes, at + 4), read_f32(bytes, at + 8))
}

fn write_i32(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(bytes: &mut [u8], at: usize, value: f32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_vec3(bytes: &mut [u8], at: usize, value: &Vec3) {
    write_f32(bytes, at, value.x);
    write_f32(bytes, at + 4, value.y);
    write_f32(bytes, at + 8, value.z);
}

/// Collision model selection (`SV_ClipHandleForEntity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmEntityCollisionModel {
    /// Inline brush model with its model index.
    Inline {
        /// Brush model index.
        index: i32,
    },
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
}

/// Owned server-side entity prefix (`sharedEntity_t.r`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmEntityShared {
    /// Whether the entity is linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client number (modern only).
    pub single_client: i32,
    /// Local bounds minimum.
    pub mins: Vec3,
    /// Local bounds maximum.
    pub maxs: Vec3,
    /// Contents mask.
    pub contents: i32,
    /// Absolute bounds minimum.
    pub absmin: Vec3,
    /// Absolute bounds maximum.
    pub absmax: Vec3,
    /// Current origin.
    pub current_origin: Vec3,
    /// Current angles.
    pub current_angles: Vec3,
    /// Owner entity number.
    pub owner_num: i32,
    /// Collision model selection.
    pub model: QvmEntityCollisionModel,
}

impl Default for QvmEntityShared {
    fn default() -> Self {
        Self {
            linked: false,
            linkcount: 0,
            sv_flags: 0,
            single_client: 0,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            contents: 0,
            absmin: vec3(0.0, 0.0, 0.0),
            absmax: vec3(0.0, 0.0, 0.0),
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            owner_num: 0,
            model: QvmEntityCollisionModel::Box,
        }
    }
}

/// Owned `sharedEntity_t` record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmSharedEntity {
    /// Entity state.
    pub s: QvmEntityState,
    /// Server-side prefix.
    pub r: QvmEntityShared,
}

impl Default for QvmSharedEntity {
    fn default() -> Self {
        Self {
            s: QvmEntityState::default(),
            r: QvmEntityShared::default(),
        }
    }
}

fn check_record(bytes: &[u8], profile: AbiProfile) -> Result<(), GuestError> {
    let need = qvm_shared_entity_bytes(profile);
    if bytes.len() < need {
        return Err(GuestError::invalid(format!(
            "QVM sharedEntity_t record requires {need} bytes, received {}",
            bytes.len()
        )));
    }
    Ok(())
}

/// Read a shared entity record.
pub fn read_qvm_shared_entity(bytes: &[u8], profile: AbiProfile) -> Result<QvmSharedEntity, GuestError> {
    check_record(bytes, profile)?;
    let at = |modern: usize| offset(profile, modern);
    let s = read_source_qvm_entity_state(bytes, profile)?;
    let sv_flags = read_i32(bytes, at(SV_FLAGS));
    let model = if read_i32(bytes, at(BMODEL)) != 0 {
        QvmEntityCollisionModel::Inline { index: s.modelindex }
    } else if sv_flags & SVF_CAPSULE != 0 {
        QvmEntityCollisionModel::Capsule
    } else {
        QvmEntityCollisionModel::Box
    };
    Ok(QvmSharedEntity {
        s,
        r: QvmEntityShared {
            linked: read_i32(bytes, at(LINKED)) != 0,
            linkcount: read_i32(bytes, at(LINKCOUNT)),
            sv_flags,
            single_client: if profile.is_modern() {
                read_i32(bytes, SINGLE_CLIENT)
            } else {
                0
            },
            mins: read_vec3(bytes, at(MINS)),
            maxs: read_vec3(bytes, at(MAXS)),
            contents: read_i32(bytes, at(CONTENTS)),
            absmin: read_vec3(bytes, at(ABSMIN)),
            absmax: read_vec3(bytes, at(ABSMAX)),
            current_origin: read_vec3(bytes, at(CURRENT_ORIGIN)),
            current_angles: read_vec3(bytes, at(CURRENT_ANGLES)),
            owner_num: read_i32(bytes, at(OWNER_NUM)),
            model,
        },
    })
}

/// Write a shared entity record, preserving the unused `r.s` gap.
pub fn write_qvm_shared_entity(
    bytes: &mut [u8],
    entity: &QvmSharedEntity,
    profile: AbiProfile,
) -> Result<(), GuestError> {
    check_record(bytes, profile)?;
    let at = |modern: usize| offset(profile, modern);
    let mut s = entity.s.clone();
    let mut sv_flags = entity.r.sv_flags;
    let bmodel = match &entity.r.model {
        QvmEntityCollisionModel::Inline { index } => {
            s.modelindex = *index;
            1
        }
        QvmEntityCollisionModel::Box => {
            sv_flags &= !SVF_CAPSULE;
            0
        }
        QvmEntityCollisionModel::Capsule => {
            sv_flags |= SVF_CAPSULE;
            0
        }
    };
    write_source_qvm_entity_state(bytes, &s, profile)?;
    write_i32(bytes, at(LINKED), i32::from(entity.r.linked));
    write_i32(bytes, at(LINKCOUNT), entity.r.linkcount);
    write_i32(bytes, at(SV_FLAGS), sv_flags);
    if profile.is_modern() {
        write_i32(bytes, SINGLE_CLIENT, entity.r.single_client);
    } else if entity.r.single_client != 0 {
        return Err(GuestError::invalid("Legacy QVM entity has no singleClient field"));
    }
    write_i32(bytes, at(BMODEL), bmodel);
    write_vec3(bytes, at(MINS), &entity.r.mins);
    write_vec3(bytes, at(MAXS), &entity.r.maxs);
    write_i32(bytes, at(CONTENTS), entity.r.contents);
    write_vec3(bytes, at(ABSMIN), &entity.r.absmin);
    write_vec3(bytes, at(ABSMAX), &entity.r.absmax);
    write_vec3(bytes, at(CURRENT_ORIGIN), &entity.r.current_origin);
    write_vec3(bytes, at(CURRENT_ANGLES), &entity.r.current_angles);
    write_i32(bytes, at(OWNER_NUM), entity.r.owner_num);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QvmSharedEntity {
        QvmSharedEntity {
            s: QvmEntityState {
                number: 3,
                modelindex: 9,
                ..QvmEntityState::default()
            },
            r: QvmEntityShared {
                linked: true,
                linkcount: 4,
                sv_flags: 8,
                single_client: 2,
                mins: vec3(-16.0, -16.0, -24.0),
                maxs: vec3(16.0, 16.0, 32.0),
                contents: 0x200_0000,
                absmin: vec3(1.0, 2.0, 3.0),
                absmax: vec3(4.0, 5.0, 6.0),
                current_origin: vec3(7.0, 8.0, 9.0),
                current_angles: vec3(10.0, 20.0, 30.0),
                owner_num: 11,
                model: QvmEntityCollisionModel::Box,
            },
        }
    }

    #[test]
    fn modern_round_trip_preserves_prefix() {
        let entity = sample();
        let mut bytes = vec![0u8; QVM_SHARED_ENTITY_BYTES];
        write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Modern).unwrap();
        assert_eq!(read_qvm_shared_entity(&bytes, AbiProfile::Modern).unwrap(), entity);
        assert_eq!(i32::from_le_bytes(bytes[512..516].try_into().unwrap()), 11);
    }

    #[test]
    fn collision_model_selection_matches_clip_handle() {
        let mut entity = sample();
        entity.r.model = QvmEntityCollisionModel::Inline { index: 5 };
        let mut bytes = vec![0u8; QVM_SHARED_ENTITY_BYTES];
        write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Modern).unwrap();
        let back = read_qvm_shared_entity(&bytes, AbiProfile::Modern).unwrap();
        assert_eq!(back.r.model, QvmEntityCollisionModel::Inline { index: 5 });
        assert_eq!(back.s.modelindex, 5);

        entity.r.model = QvmEntityCollisionModel::Capsule;
        write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Modern).unwrap();
        let back = read_qvm_shared_entity(&bytes, AbiProfile::Modern).unwrap();
        assert_eq!(back.r.model, QvmEntityCollisionModel::Capsule);
        assert_ne!(back.r.sv_flags & SVF_CAPSULE, 0);

        entity.r.model = QvmEntityCollisionModel::Box;
        write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Modern).unwrap();
        let back = read_qvm_shared_entity(&bytes, AbiProfile::Modern).unwrap();
        assert_eq!(back.r.model, QvmEntityCollisionModel::Box);
        assert_eq!(back.r.sv_flags & SVF_CAPSULE, 0);
    }

    #[test]
    fn legacy_layout_round_trips() {
        let mut entity = sample();
        entity.r.single_client = 0;
        let mut bytes = vec![0u8; 504];
        write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Legacy).unwrap();
        let back = read_qvm_shared_entity(&bytes, AbiProfile::Legacy).unwrap();
        assert_eq!(back.r.owner_num, 11);
        assert_eq!(back.r.linkcount, 4);
        assert!(back.r.linked);
        entity.r.single_client = 1;
        assert!(write_qvm_shared_entity(&mut bytes, &entity, AbiProfile::Legacy).is_err());
    }

    #[test]
    fn unused_gap_survives_writes() {
        let mut bytes = vec![0u8; QVM_SHARED_ENTITY_BYTES];
        bytes[300] = 0xAB;
        write_qvm_shared_entity(&mut bytes, &sample(), AbiProfile::Modern).unwrap();
        assert_eq!(bytes[300], 0xAB);
    }
}
