//! Quake III base/shared: snapshot state.
//!
//! Donor provenance: `src/content/q3/base/shared/snapshot-state.ts`.

use qa_core::math::{vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions_mirror::*;
use crate::q3::base::shared::entity_state::*;
use crate::q3::base::shared::player_state::*;
use crate::q3::base::shared::trajectory::*;

// ---------------------------------------------------------------------------
// shared/snapshot-state.ts
// ---------------------------------------------------------------------------

// `q_shared.h`'s `SnapVector` casts to int, unlike `Sys_SnapVector`'s
// nearest rounding. The out-of-range conversion matches the native x86
// source's indefinite integer.
pub(crate) fn source_snap_component(component: f32) -> f32 {
    if (-2147483648.0..2147483648.0).contains(&component) {
        component.trunc() + 0.0
    } else {
        -2147483648.0
    }
}

pub(crate) fn copy_position(value: Vec3, snap: bool) -> Vec3 {
    if snap {
        vec3(
            source_snap_component(value.x),
            source_snap_component(value.y),
            source_snap_component(value.z),
        )
    } else {
        value
    }
}

pub(crate) fn convert_player_state(
    ps: &mut SourcePlayerState,
    s: &mut EntityState,
    snap: bool,
    extrapolation_time: Option<i32>,
) {
    s.e_type = if ps.pm_type == MoveType::PmIntermission as i32
        || ps.pm_type == MoveType::PmSpectator as i32
        || ps.health() <= GIB_HEALTH
    {
        EntityType::EtInvisible as i32
    } else {
        EntityType::EtPlayer as i32
    };
    s.number = ps.client_num;
    s.pos = Trajectory {
        trajectory_type: if extrapolation_time.is_none() {
            TrajectoryType::TrInterpolate
        } else {
            TrajectoryType::TrLinearStop
        },
        base: copy_position(ps.origin(), snap),
        delta: ps.velocity(),
        time: extrapolation_time.unwrap_or(s.pos.time),
        duration: if extrapolation_time.is_none() {
            s.pos.duration
        } else {
            50
        },
    };
    s.apos.trajectory_type = TrajectoryType::TrInterpolate;
    s.apos.base = copy_position(ps.viewangles, snap);
    s.angles2.y = ps.movement_dir as f32;
    s.legs_anim = ps.legs_anim;
    s.torso_anim = ps.torso_anim;
    s.client_num = ps.client_num;
    s.e_flags = if ps.health() <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };

    if ps.external_event != 0 {
        s.event = ps.external_event;
        s.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let slot = (ps.entity_event_sequence & 1) as usize;
        s.event = ps.events.get(slot) | ((ps.entity_event_sequence & 3) << 8);
        s.event_parm = ps.event_parms.get(slot);
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }

    s.weapon = ps.weapon;
    s.ground_entity_num = ps.ground_entity_num;
    s.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index) != 0 {
            s.powerups |= 1 << index;
        }
    }
    s.loop_sound = ps.loop_sound;
    s.generic1 = ps.generic1;
}

/// Convert a player state to an entity state
/// (`BG_PlayerStateToEntityState`).
///
/// Updates source-owned fields and consumes one pending predictable event.
pub fn player_state_to_entity_state(ps: &mut SourcePlayerState, destination: &mut EntityState, snap: bool) {
    convert_player_state(ps, destination, snap, None);
}

/// Convert a player state to an extrapolated entity state
/// (`BG_PlayerStateToEntityStateExtraPolate`).
///
/// Publishes at most 50ms of linear extrapolation, matching the source's
/// fixed duration.
pub fn player_state_to_entity_state_extra_polate(
    ps: &mut SourcePlayerState,
    destination: &mut EntityState,
    time: i32,
    snap: bool,
) {
    convert_player_state(ps, destination, snap, Some(time));
}
