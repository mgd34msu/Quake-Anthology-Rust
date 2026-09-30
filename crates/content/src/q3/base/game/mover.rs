//! Quake III base/game: mover.
//!
//! Donor provenance: `src/content/q3/base/game/mover.ts`.

use qa_core::identity::ActorId;
use qa_core::math::add3;
use qa_core::math::angle_vectors;
use qa_core::math::dot3;
use qa_core::math::length3;
use qa_core::math::radius_from_bounds;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::spawn::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{
    failure, lose_ground, rides, run_think, EntityPool, Q3BodyState, Q3Driver, Q3GameError,
};
use crate::q3::base::shared::definitions::{EntityEvent, EntityType, ItemType, Product};
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::base::shared::player_state::ENTITYNUM_WORLD;
use crate::q3::base::shared::trajectory::{evaluate_trajectory, Trajectory, TrajectoryType};
use crate::q3::base::world::{ActorTraceHit, ActorTraceQuery, TraceShape, TraceSolidity};

// ---------------------------------------------------------------------------
// mover.ts: push transactions and binary movers (g_mover.c)
// ---------------------------------------------------------------------------

/// Mover-stop effect flag (`EF_MOVER_STOP`).
pub(crate) const EF_MOVER_STOP: i32 = 0x400;

/// Crush means of death (`MOD_CRUSH`).
pub(crate) const MOD_CRUSH: i32 = 17;

/// Convert a fallible runtime result into a fail-fast panic at native-callback
/// boundaries, matching donor throws inside callbacks.
pub(crate) fn or_panic(result: Result<(), Q3GameError>) {
    if let Err(error) = result {
        panic!("{error}");
    }
}

/// Pushed-entity stack entry.
#[derive(Debug, Clone)]
pub(crate) enum PushedEntity {
    /// Native entity.
    Native {
        /// Slot.
        slot: usize,
        /// Saved origin.
        origin: Vec3,
        /// Saved angles.
        angles: Vec3,
        /// Saved client yaw, when the entity had a client.
        yaw: Option<f32>,
    },
    /// Shared body.
    Shared {
        /// Body snapshot.
        body: SharedMoverBody,
    },
}

/// Pusher snapshot for push transactions.
#[derive(Debug, Clone)]
pub(crate) struct PusherSnapshot {
    actor: ActorId,
    e_flags: i32,
    current_origin: Vec3,
    current_angles: Vec3,
    mins: Vec3,
    maxs: Vec3,
    absmin: Vec3,
    absmax: Vec3,
    pos_type: TrajectoryType,
    apos_type: TrajectoryType,
}

pub(crate) fn mover_snapshot(pool: &dyn EntityPool, slot: usize) -> Result<PusherSnapshot, Q3GameError> {
    let Some(entity) = pool.entity(slot) else {
        return Err(failure("Mover entity does not belong to this pool"));
    };
    Ok(PusherSnapshot {
        actor: entity.actor.clone(),
        e_flags: entity.s.e_flags,
        current_origin: entity.r.current_origin,
        current_angles: entity.r.current_angles,
        mins: entity.r.mins,
        maxs: entity.r.maxs,
        absmin: entity.r.absmin(),
        absmax: entity.r.absmax(),
        pos_type: entity.s.pos.trajectory_type,
        apos_type: entity.s.apos.trajectory_type,
    })
}

pub(crate) fn mover_angle_short(angle: f32) -> i32 {
    qvm_float_to_int(angle * 65536.0 / 360.0) & 65535
}

pub(crate) fn push_rotation(origin: Vec3, pusher_origin: Vec3, amove: Vec3) -> Vec3 {
    let axes = angle_vectors(amove);
    let m0 = axes.forward;
    let m1 = scale3(axes.right, -1.0);
    let m2 = axes.up;
    let org = sub3(origin, pusher_origin);
    let rotated = vec3(
        dot3(org, vec3(m0.x, m1.x, m2.x)),
        dot3(org, vec3(m0.y, m1.y, m2.y)),
        dot3(org, vec3(m0.z, m1.z, m2.z)),
    );
    sub3(rotated, org)
}

/// Mover runtime (`MoverRuntime`).
#[derive(Debug, Clone)]
pub struct MoverRuntime {
    /// Previous frame time.
    pub previous_time: i32,
}

impl MoverRuntime {
    /// New runtime.
    #[must_use]
    pub fn new(previous_time: i32) -> Self {
        Self { previous_time }
    }

    fn check_product(&self, driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
        if driver.pool().product() != driver.combat().product() {
            return Err(failure("Mover product does not match its entity pool"));
        }
        Ok(())
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure("Mover entity does not belong to this pool"));
        }
        Ok(())
    }

    /// Test an entity position (`testEntityPosition`).
    pub fn test_entity_position(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
    ) -> Result<Option<Participant>, Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let (client_slot, pos_base, mins, maxs, actor, clipmask) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (
                entity.client,
                entity.s.pos.base,
                entity.r.mins,
                entity.r.maxs,
                entity.actor.clone(),
                entity.clipmask,
            )
        };
        let start = match client_slot {
            None => pos_base,
            Some(client) => driver
                .pool()
                .client(client)
                .map(|client| client.ps.origin)
                .unwrap_or(pos_base),
        };
        let query = ActorTraceQuery {
            start,
            end: start,
            shape: TraceShape::Box { mins, maxs },
            pass_actor: Some(actor),
            mask: if clipmask == 0 { 1 } else { clipmask },
        };
        let result = driver.spatial().trace_actor(&query);
        if result.solidity == TraceSolidity::Clear {
            return Ok(None);
        }
        if let ActorTraceHit::Actor { actor } = &result.hit {
            let participant = driver.mover_actors().participant(actor);
            return Ok(Some(participant));
        }
        self.owned(driver, ENTITYNUM_WORLD as usize)?;
        Ok(Some(Participant::Entity(ENTITYNUM_WORLD as usize)))
    }

    fn try_pushing(
        &self,
        driver: &mut dyn Q3Driver,
        check: usize,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<bool, Q3GameError> {
        let rider = driver
            .pool()
            .entity(check)
            .is_some_and(|entity| rides(entity, &pusher.actor));
        if pusher.e_flags & EF_MOVER_STOP != 0 && !rider {
            return Ok(false);
        }
        if pushed.len() >= MAX_GENTITIES {
            return Err(failure("pushed stack exceeds MAX_GENTITIES"));
        }
        let (client_slot, pos_base, apos_base) = {
            let entity = driver
                .pool()
                .entity(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.client, entity.s.pos.base, entity.s.apos.base)
        };
        let origin = match client_slot {
            None => pos_base,
            Some(client) => driver
                .pool()
                .client(client)
                .map(|client| client.ps.origin)
                .unwrap_or(pos_base),
        };
        let yaw = client_slot.and_then(|client| {
            driver
                .pool()
                .client(client)
                .map(|client| client.ps.delta_angles[1] as f32)
        });
        let saved = PushedEntity::Native {
            slot: check,
            origin,
            angles: apos_base,
            yaw,
        };
        pushed.push(saved.clone());
        let PushedEntity::Native {
            origin: saved_origin, ..
        } = &saved
        else {
            unreachable!()
        };
        let rotation = push_rotation(*saved_origin, pusher.current_origin, amove);
        let client_slot = {
            let entity = driver
                .pool()
                .entity_mut(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.s.pos.base = add3(add3(entity.s.pos.base, mv), rotation);
            entity.client
        };
        if let Some(client) = client_slot {
            let yaw_add = mover_angle_short(amove.y);
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.origin = add3(add3(client.ps.origin, mv), rotation);
                client.ps.delta_angles[1] = client.ps.delta_angles[1].wrapping_add(yaw_add);
            }
        }
        let still_rider = driver
            .pool()
            .entity(check)
            .is_some_and(|entity| rides(entity, &pusher.actor));
        if !still_rider {
            if let Some(entity) = driver.pool().entity_mut(check) {
                lose_ground(entity);
            }
        }
        if self.test_entity_position(driver, check)?.is_none() {
            let (client_slot, pos_base) = {
                let entity = driver
                    .pool()
                    .entity(check)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.client, entity.s.pos.base)
            };
            let origin = match client_slot {
                None => pos_base,
                Some(client) => driver
                    .pool()
                    .client(client)
                    .map(|client| client.ps.origin)
                    .unwrap_or(pos_base),
            };
            if let Some(entity) = driver.pool().entity_mut(check) {
                entity.r.current_origin = origin;
            }
            driver.world().link(check);
            return Ok(true);
        }
        let saved_angles = match &saved {
            PushedEntity::Native { angles, .. } => *angles,
            PushedEntity::Shared { .. } => unreachable!("pushed native entry"),
        };
        let client_slot = {
            let entity = driver
                .pool()
                .entity_mut(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.s.pos.base = *saved_origin;
            entity.s.apos.base = saved_angles;
            entity.client
        };
        if let Some(client) = client_slot {
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.origin = *saved_origin;
            }
        }
        if self.test_entity_position(driver, check)?.is_none() {
            if let Some(entity) = driver.pool().entity_mut(check) {
                lose_ground(entity);
            }
            pushed.pop();
            return Ok(true);
        }
        Ok(false)
    }

    fn check_proximity_position(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<bool, Q3GameError> {
        let (pos_base, movedir, actor) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.s.pos.base, entity.movedir, entity.actor.clone())
        };
        let start = add3(pos_base, scale3(movedir, 0.125));
        let end = add3(pos_base, scale3(movedir, 2.0));
        let query = ActorTraceQuery {
            start,
            end,
            shape: TraceShape::Point,
            pass_actor: Some(actor),
            mask: 1,
        };
        let trace = driver.spatial().trace_actor(&query);
        Ok(trace.solidity == TraceSolidity::Clear && trace.fraction == 1.0)
    }

    fn shared_position_blocked(&self, driver: &mut dyn Q3Driver, check: &SharedMoverBody) -> bool {
        let Some(body) = driver.mover_actors().observe(&check.actor) else {
            return false;
        };
        let query = ActorTraceQuery {
            start: body.state.origin,
            end: body.state.origin,
            shape: TraceShape::Box {
                mins: body.state.bounds.min,
                maxs: body.state.bounds.max,
            },
            pass_actor: Some(check.actor.clone()),
            mask: if check.clip_mask == 0 { 1 } else { check.clip_mask },
        };
        driver.spatial().trace_actor(&query).solidity != TraceSolidity::Clear
    }

    fn try_pushing_shared(
        &self,
        driver: &mut dyn Q3Driver,
        check: &SharedMoverBody,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<bool, Q3GameError> {
        let rider = check
            .state
            .ground
            .as_ref()
            .is_some_and(|ground| ground == &pusher.actor);
        if pusher.e_flags & EF_MOVER_STOP != 0 && !rider {
            return Ok(false);
        }
        if pushed.len() >= MAX_GENTITIES {
            return Err(failure("pushed stack exceeds MAX_GENTITIES"));
        }
        pushed.push(PushedEntity::Shared { body: check.clone() });
        let rotation = push_rotation(check.state.origin, pusher.current_origin, amove);
        let ground = if rider { check.state.ground.clone() } else { None };
        driver.mover_actors().write(
            &check.actor,
            add3(add3(check.state.origin, mv), rotation),
            ground.clone(),
        );
        if !self.shared_position_blocked(driver, check) {
            driver.mover_actors().link_actor(&check.actor);
            return Ok(true);
        }
        driver.mover_actors().write(&check.actor, check.state.origin, ground);
        if !self.shared_position_blocked(driver, check) {
            driver.mover_actors().write(&check.actor, check.state.origin, None);
            pushed.pop();
            return Ok(true);
        }
        Ok(false)
    }

    fn restore_pushed(&self, driver: &mut dyn Q3Driver, pushed: &[PushedEntity]) -> Result<(), Q3GameError> {
        for saved in pushed.iter().rev() {
            match saved {
                PushedEntity::Shared { body } => {
                    if let Some(current) = driver.mover_actors().observe(&body.actor) {
                        driver
                            .mover_actors()
                            .write(&body.actor, body.state.origin, current.state.ground.clone());
                        driver.mover_actors().link_actor(&body.actor);
                    }
                }
                PushedEntity::Native {
                    slot,
                    origin,
                    angles,
                    yaw,
                } => {
                    let slot = *slot;
                    let has_client = driver.pool().entity(slot).and_then(|entity| entity.client);
                    if let Some(entity) = driver.pool().entity_mut(slot) {
                        entity.s.pos.base = *origin;
                        entity.s.apos.base = *angles;
                    }
                    if let Some(client) = has_client {
                        let Some(yaw) = yaw else {
                            return Err(failure("pushed entity acquired a client during the transaction"));
                        };
                        if let Some(client) = driver.pool().client_mut(client) {
                            client.ps.delta_angles[1] = qvm_float_to_int(*yaw);
                            client.ps.origin = *origin;
                        }
                    }
                    driver.world().link(slot);
                }
            }
        }
        Ok(())
    }

    fn push_proximity_mine(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
    ) -> Result<bool, Q3GameError> {
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            let axes = angle_vectors(sub3(vec3(0.0, 0.0, 0.0), amove));
            entity.s.pos.base = add3(entity.s.pos.base, mv);
            let org = sub3(entity.s.pos.base, pusher.current_origin);
            let rotated = vec3(dot3(org, axes.forward), -dot3(org, axes.right), dot3(org, axes.up));
            entity.s.pos.base = add3(entity.s.pos.base, sub3(rotated, org));
        }
        if !self.check_proximity_position(driver, slot)? {
            return Ok(false);
        }
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.r.current_origin = entity.s.pos.base;
        }
        driver.world().link(slot);
        Ok(true)
    }

    fn push_part(
        &self,
        driver: &mut dyn Q3Driver,
        pusher_slot: usize,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<Option<Participant>, Q3GameError> {
        let pusher = mover_snapshot(driver.pool(), pusher_slot)?;
        let rotating = pusher.current_angles.x != 0.0
            || pusher.current_angles.y != 0.0
            || pusher.current_angles.z != 0.0
            || amove.x != 0.0
            || amove.y != 0.0
            || amove.z != 0.0;
        let (destination, total) = if rotating {
            let radius = radius_from_bounds(Bounds {
                min: pusher.mins,
                max: pusher.maxs,
            });
            let extent = vec3(radius, radius, radius);
            let position = add3(pusher.current_origin, mv);
            let destination = Bounds {
                min: sub3(position, extent),
                max: add3(position, extent),
            };
            let total = Bounds {
                min: sub3(destination.min, mv),
                max: sub3(destination.max, mv),
            };
            (destination, total)
        } else {
            let bounds = Bounds {
                min: pusher.absmin,
                max: pusher.absmax,
            };
            let destination = Bounds {
                min: add3(bounds.min, mv),
                max: add3(bounds.max, mv),
            };
            let total = Bounds {
                min: add3(bounds.min, vec3(mv.x.min(0.0), mv.y.min(0.0), mv.z.min(0.0))),
                max: add3(bounds.max, vec3(mv.x.max(0.0), mv.y.max(0.0), mv.z.max(0.0))),
            };
            (destination, total)
        };
        driver.world().unlink(pusher_slot);
        let actors = driver.spatial().area_actors(&total, MAX_GENTITIES);
        {
            let entity = driver
                .pool()
                .entity_mut(pusher_slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.r.current_origin = add3(entity.r.current_origin, mv);
            entity.r.current_angles = add3(entity.r.current_angles, amove);
        }
        driver.world().link(pusher_slot);
        let pusher = mover_snapshot(driver.pool(), pusher_slot)?;
        for actor in actors {
            let observed = driver.mover_actors().observe(&actor);
            let Some(observed) = observed else { continue };
            if observed.kind == SharedBodyKind::Attached {
                continue;
            }
            let native = driver.mover_actors().native_slot(&actor);
            if native.is_none() {
                if observed.kind == SharedBodyKind::Fixed {
                    continue;
                }
                let rider = observed
                    .state
                    .ground
                    .as_ref()
                    .is_some_and(|ground| ground == &pusher.actor);
                if !rider {
                    let bounds = observed.absolute_bounds;
                    if bounds.min.x >= destination.max.x
                        || bounds.min.y >= destination.max.y
                        || bounds.min.z >= destination.max.z
                        || bounds.max.x <= destination.min.x
                        || bounds.max.y <= destination.min.y
                        || bounds.max.z <= destination.min.z
                    {
                        continue;
                    }
                    if !self.shared_position_blocked(driver, &observed) {
                        continue;
                    }
                }
                if self.try_pushing_shared(driver, &observed, &pusher, mv, amove, pushed)? {
                    continue;
                }
                let participant = driver.mover_actors().participant(&actor);
                if pusher.pos_type == TrajectoryType::TrSine || pusher.apos_type == TrajectoryType::TrSine {
                    let host = Participant::Entity(pusher_slot);
                    driver
                        .combat()
                        .damage(&participant, Some(&host), Some(&host), None, None, 99999, 0, MOD_CRUSH);
                    continue;
                }
                self.restore_pushed(driver, pushed)?;
                return Ok(Some(participant));
            }
            let check = native.unwrap_or(usize::MAX);
            let is_missionpack = driver.combat().product() == Product::Missionpack;
            if is_missionpack {
                let (e_type, classname, enemy) = match driver.pool().entity(check) {
                    Some(entity) => (
                        entity.s.e_type,
                        entity.classname_value().map(str::to_string),
                        entity.enemy,
                    ),
                    None => continue,
                };
                if e_type == EntityType::EtMissile as i32 && classname.as_deref() == Some("prox mine") {
                    let clear = if enemy == Some(pusher_slot) {
                        self.push_proximity_mine(driver, check, &pusher, mv, amove)?
                    } else {
                        self.check_proximity_position(driver, check)?
                    };
                    if !clear {
                        if let Some(entity) = driver.pool().entity_mut(check) {
                            entity.s.loop_sound = 0;
                        }
                        driver.pool().add_event(check, EntityEvent::EvProximityMineTrigger, 0);
                        driver.explode_missile(check);
                        let activator = driver.pool().entity(check).and_then(|entity| entity.activator);
                        if let Some(activator) = activator {
                            driver.pool().free_entity(activator);
                            if let Some(entity) = driver.pool().entity_mut(check) {
                                entity.activator = None;
                            }
                        }
                    }
                    continue;
                }
            }
            let (e_type, physics_object) = match driver.pool().entity(check) {
                Some(entity) => (entity.s.e_type, entity.physics_object),
                None => continue,
            };
            if e_type != EntityType::EtItem as i32 && e_type != EntityType::EtPlayer as i32 && !physics_object {
                continue;
            }
            let rider = driver
                .pool()
                .entity(check)
                .is_some_and(|entity| rides(entity, &pusher.actor));
            if !rider {
                let inside = match driver.pool().entity(check) {
                    Some(entity) => {
                        let bounds = Bounds {
                            min: entity.r.absmin(),
                            max: entity.r.absmax(),
                        };
                        bounds.min.x < destination.max.x
                            && bounds.min.y < destination.max.y
                            && bounds.min.z < destination.max.z
                            && bounds.max.x > destination.min.x
                            && bounds.max.y > destination.min.y
                            && bounds.max.z > destination.min.z
                    }
                    None => false,
                };
                if !inside {
                    continue;
                }
                if self.test_entity_position(driver, check)?.is_none() {
                    continue;
                }
            }
            if self.try_pushing(driver, check, &pusher, mv, amove, pushed)? {
                continue;
            }
            if pusher.pos_type == TrajectoryType::TrSine || pusher.apos_type == TrajectoryType::TrSine {
                let target = Participant::Entity(check);
                let host = Participant::Entity(pusher_slot);
                driver
                    .combat()
                    .damage(&target, Some(&host), Some(&host), None, None, 99999, 0, MOD_CRUSH);
                continue;
            }
            self.restore_pushed(driver, pushed)?;
            return Ok(Some(Participant::Entity(check)));
        }
        Ok(None)
    }

    /// Run a mover team (`runTeam`).
    pub fn run_team(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let mut pushed = Vec::new();
        let time = driver.combat().time();
        let mut obstacle = None;
        let mut part = Some(slot);
        while let Some(current) = part {
            let (pos, apos, origin, angles) = {
                let entity = driver
                    .pool()
                    .entity(current)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (
                    entity.s.pos,
                    entity.s.apos,
                    entity.r.current_origin,
                    entity.r.current_angles,
                )
            };
            let mv = sub3(evaluate_trajectory(&pos, time), origin);
            let amove = sub3(evaluate_trajectory(&apos, time), angles);
            obstacle = self.push_part(driver, current, mv, amove, &mut pushed)?;
            if obstacle.is_some() {
                break;
            }
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        if let Some(obstacle) = obstacle {
            let elapsed = time.wrapping_sub(self.previous_time);
            let mut part = Some(slot);
            while let Some(current) = part {
                {
                    let entity = driver
                        .pool()
                        .entity_mut(current)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    entity.s.pos.time = entity.s.pos.time.wrapping_add(elapsed);
                    entity.s.apos.time = entity.s.apos.time.wrapping_add(elapsed);
                    entity.r.current_origin = evaluate_trajectory(&entity.s.pos, time);
                    entity.r.current_angles = evaluate_trajectory(&entity.s.apos, time);
                }
                driver.world().link(current);
                part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
            }
            let blocked = driver.pool().entity(slot).and_then(|entity| entity.blocked.clone());
            if let Some(blocked) = blocked {
                blocked(driver, slot, &obstacle);
            }
            return Ok(());
        }
        let mut part = Some(slot);
        while let Some(current) = part {
            let (pos_type, pos_time, pos_duration, reached) = {
                let entity = driver
                    .pool()
                    .entity(current)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (
                    entity.s.pos.trajectory_type,
                    entity.s.pos.time,
                    entity.s.pos.duration,
                    entity.reached.clone(),
                )
            };
            if pos_type == TrajectoryType::TrLinearStop && time >= pos_time.wrapping_add(pos_duration) {
                if let Some(reached) = reached {
                    reached(driver, current);
                }
            }
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        Ok(())
    }

    /// Run a mover (`run`).
    pub fn run(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
        if entity.flags & GameFlags::TEAMSLAVE != 0 {
            return Ok(());
        }
        let moving = entity.s.pos.trajectory_type != TrajectoryType::TrStationary
            || entity.s.apos.trajectory_type != TrajectoryType::TrStationary;
        if moving {
            self.run_team(driver, slot)?;
        }
        let time = driver.combat().time();
        run_think(driver, slot, time)
    }

    /// Set a mover state (`setState`).
    pub fn set_state(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        state: MoverState,
        time: i32,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let now = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.mover_state = state as i32;
            let previous = entity.s.pos;
            entity.s.pos = match state {
                MoverState::Pos1 => Trajectory {
                    trajectory_type: TrajectoryType::TrStationary,
                    time,
                    base: entity.pos1,
                    ..previous
                },
                MoverState::Pos2 => Trajectory {
                    trajectory_type: TrajectoryType::TrStationary,
                    time,
                    base: entity.pos2,
                    ..previous
                },
                MoverState::OneToTwo => Trajectory {
                    trajectory_type: TrajectoryType::TrLinearStop,
                    time,
                    base: entity.pos1,
                    delta: scale3(sub3(entity.pos2, entity.pos1), 1000.0 / previous.duration as f32),
                    ..previous
                },
                MoverState::TwoToOne => Trajectory {
                    trajectory_type: TrajectoryType::TrLinearStop,
                    time,
                    base: entity.pos2,
                    delta: scale3(sub3(entity.pos1, entity.pos2), 1000.0 / previous.duration as f32),
                    ..previous
                },
            };
            entity.r.current_origin = evaluate_trajectory(&entity.s.pos, now);
        }
        driver.world().link(slot);
        Ok(())
    }

    /// Match a team to a state (`matchTeam`).
    pub fn match_team(
        &self,
        driver: &mut dyn Q3Driver,
        leader: usize,
        state: MoverState,
        time: i32,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, leader)?;
        let mut part = Some(leader);
        while let Some(current) = part {
            self.set_state(driver, current, state, time)?;
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        Ok(())
    }

    /// Return a mover to position 1 (`returnToPos1`).
    pub fn return_to_pos1(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        let time = driver.combat().time();
        self.match_team(driver, slot, MoverState::TwoToOne, time)?;
        let (sound_loop, sound2to1) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.sound_loop, entity.sound2to1)
        };
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = sound_loop;
        }
        if sound2to1 != 0 {
            driver.pool().add_event(slot, EntityEvent::EvGeneralSound, sound2to1);
        }
        Ok(())
    }

    /// Binary-mover reached handler (`reachedBinary`).
    pub fn reached_binary(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let time = driver.combat().time();
        let sound_loop = driver.pool().entity(slot).map(|entity| entity.sound_loop).unwrap_or(0);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = sound_loop;
        }
        let mover_state = driver.pool().entity(slot).map(|entity| entity.mover_state).unwrap_or(0);
        if mover_state == MoverState::OneToTwo as i32 {
            self.set_state(driver, slot, MoverState::Pos2, time)?;
            let (sound_pos2, wait) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound_pos2, entity.wait)
            };
            if sound_pos2 != 0 {
                driver.pool().add_event(slot, EntityEvent::EvGeneralSound, sound_pos2);
            }
            let think = driver
                .pool()
                .callbacks()
                .think
                .resolve(Some("q3.base.game.mover.reachedBinary.think"))?;
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.think = think;
            }
            driver.pool().set_nextthink(slot, qvm_float_to_int(time as f32 + wait));
            let activation_missing = driver
                .pool()
                .entity(slot)
                .is_some_and(|entity| entity.activation.is_none());
            if activation_missing {
                if let Some(entity) = driver.pool().entity_mut(slot) {
                    entity.activation = Some(Participant::Entity(slot));
                }
            }
            let activation = driver.pool().entity(slot).and_then(|entity| entity.activation.clone());
            driver.use_targets(slot, activation);
            Ok(())
        } else if mover_state == MoverState::TwoToOne as i32 {
            self.set_state(driver, slot, MoverState::Pos1, time)?;
            let (sound_pos1, teammaster) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound_pos1, entity.teammaster)
            };
            if sound_pos1 != 0 {
                driver.pool().add_event(slot, EntityEvent::EvGeneralSound, sound_pos1);
            }
            if teammaster.is_none() || teammaster == Some(slot) {
                driver.adjust_area_portal(slot, false);
            }
            Ok(())
        } else {
            Err(failure("Reached_BinaryMover: bad moverState"))
        }
    }

    /// Binary-mover use handler (`useBinary`).
    #[allow(clippy::only_used_in_recursion)]
    pub fn use_binary(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        other: Option<Participant>,
        activator: Option<Participant>,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let flags = driver.pool().entity(slot).map(|entity| entity.flags).unwrap_or(0);
        if flags & GameFlags::TEAMSLAVE != 0 {
            let master = driver.pool().entity(slot).and_then(|entity| entity.teammaster);
            let Some(master) = master else {
                return Err(failure("Mover team slave has no team master"));
            };
            return self.use_binary(driver, master, other, activator);
        }
        let time = driver.combat().time();
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = activator;
        }
        let mover_state = driver.pool().entity(slot).map(|entity| entity.mover_state).unwrap_or(0);
        if mover_state == MoverState::Pos1 as i32 {
            self.match_team(driver, slot, MoverState::OneToTwo, time.wrapping_add(50))?;
            let (sound1to2, sound_loop, teammaster) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound1to2, entity.sound_loop, entity.teammaster)
            };
            if sound1to2 != 0 {
                driver.pool().add_event(slot, EntityEvent::EvGeneralSound, sound1to2);
            }
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.loop_sound = sound_loop;
            }
            if teammaster.is_none() || teammaster == Some(slot) {
                driver.adjust_area_portal(slot, true);
            }
            Ok(())
        } else if mover_state == MoverState::Pos2 as i32 {
            let wait = driver.pool().entity(slot).map(|entity| entity.wait).unwrap_or(0.0);
            driver.pool().set_nextthink(slot, qvm_float_to_int(time as f32 + wait));
            Ok(())
        } else {
            let (duration, pos_time) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.s.pos.duration, entity.s.pos.time)
            };
            let partial = (time.wrapping_sub(pos_time)).min(duration);
            let state = if mover_state == MoverState::TwoToOne as i32 {
                MoverState::OneToTwo
            } else {
                MoverState::TwoToOne
            };
            self.match_team(driver, slot, state, time.wrapping_sub(duration.wrapping_sub(partial)))?;
            let sound = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                if state == MoverState::OneToTwo {
                    entity.sound1to2
                } else {
                    entity.sound2to1
                }
            };
            if sound != 0 {
                driver.pool().add_event(slot, EntityEvent::EvGeneralSound, sound);
            }
            Ok(())
        }
    }

    /// Initialize a binary mover (`initializeBinary`).
    pub fn initialize_binary(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        variables: &SpawnVariables,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let model2 = driver.pool().entity(slot).and_then(|entity| entity.model2.clone());
        if let Some(model2) = model2 {
            let index = driver.model_index(Some(&model2));
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.modelindex2 = index;
            }
        }
        let noise = variables.string("noise", "100");
        if noise.present {
            let index = driver.sound_index(&noise.value);
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.loop_sound = index;
            }
        }
        let light = variables.float("light", "100")?;
        let color = variables.vector("color", "1 1 1")?;
        if light.present || color.present {
            let component = |value: f32| -> i32 { 255.min(qvm_float_to_int(value * 255.0)) };
            let intensity = 255.min(qvm_float_to_int(light.value / 4.0));
            let packed = component(color.value.x)
                | (component(color.value.y) << 8)
                | (component(color.value.z) << 16)
                | (intensity << 24);
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.constant_light = packed;
            }
        }
        let use_callback = driver
            .pool()
            .callbacks()
            .use_callbacks
            .resolve(Some("q3.base.game.mover.initializeBinary.use"))?;
        let reached = driver
            .pool()
            .callbacks()
            .reached
            .resolve(Some("q3.base.game.mover.initializeBinary.reached"))?;
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.use_callback = use_callback;
            entity.reached = reached;
            entity.mover_state = MoverState::Pos1 as i32;
            entity.r.sv_flags = ServerEntityFlags::UseCurrentOrigin as i32;
            entity.s.e_type = EntityType::EtMover as i32;
            entity.r.current_origin = entity.pos1;
        }
        driver.world().link(slot);
        let (pos1, pos2, speed) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.pos1, entity.pos2, entity.speed)
        };
        let distance = length3(sub3(pos2, pos1));
        let speed = if speed == 0.0 { 100.0 } else { speed };
        let duration = qvm_float_to_int(distance * 1000.0 / speed).max(1);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.speed = speed;
            let previous = entity.s.pos;
            entity.s.pos = Trajectory {
                trajectory_type: TrajectoryType::TrStationary,
                base: entity.pos1,
                delta: scale3(sub3(entity.pos2, entity.pos1), speed),
                duration,
                ..previous
            };
        }
        Ok(())
    }

    /// Door blocked handler (`blockedDoor`).
    pub fn blocked_door(&self, driver: &mut dyn Q3Driver, slot: usize, other: &Participant) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        match other {
            Participant::SharedActor(actor) => {
                let body = driver.mover_actors().observe(actor);
                let Some(body) = body else { return Ok(()) };
                if body.kind != SharedBodyKind::Player {
                    driver.pool().temp_entity(body.state.origin, EntityEvent::EvItemPop);
                    driver.mover_actors().release(&body.actor);
                    return Ok(());
                }
                let (damage, spawnflags) = {
                    let entity = driver
                        .pool()
                        .entity(slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.damage, entity.spawnflags)
                };
                if damage != 0 {
                    let host = Participant::Entity(slot);
                    driver
                        .combat()
                        .damage(other, Some(&host), Some(&host), None, None, damage, 0, MOD_CRUSH);
                }
                if spawnflags & 4 == 0 {
                    self.use_binary(driver, slot, Some(Participant::Entity(slot)), Some(other.clone()))?;
                }
                Ok(())
            }
            Participant::Entity(other_slot) => {
                let other_slot = *other_slot;
                self.owned(driver, other_slot)?;
                let (client, e_type, origin, item) = {
                    let entity = driver
                        .pool()
                        .entity(other_slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.client, entity.s.e_type, entity.s.origin, entity.item)
                };
                if client.is_none() {
                    if e_type == EntityType::EtItem as i32 {
                        let Some(item) = item else {
                            return Err(failure("Blocked item has no item definition"));
                        };
                        let def = driver.item_at(item);
                        if def.is_none() {
                            return Err(failure("Blocked item has no item definition"));
                        }
                        if def.is_some_and(|def| def.item_type == ItemType::ItTeam) {
                            driver.return_dropped_flag(other_slot);
                            return Ok(());
                        }
                    }
                    driver.pool().temp_entity(origin, EntityEvent::EvItemPop);
                    driver.pool().free_entity(other_slot);
                    return Ok(());
                }
                let (damage, spawnflags) = {
                    let entity = driver
                        .pool()
                        .entity(slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.damage, entity.spawnflags)
                };
                if damage != 0 {
                    let host = Participant::Entity(slot);
                    driver
                        .combat()
                        .damage(other, Some(&host), Some(&host), None, None, damage, 0, MOD_CRUSH);
                }
                if spawnflags & 4 != 0 {
                    return Ok(());
                }
                self.use_binary(driver, slot, Some(Participant::Entity(slot)), Some(other.clone()))
            }
        }
    }

    /// Bind save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<MoverRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let think_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.mover.reachedBinary.think",
            Rc::new(move |driver, slot| {
                let result = think_runtime.borrow_mut().return_to_pos1(driver, slot);
                or_panic(result);
            }),
        )?;
        let use_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().use_callbacks.intern(
            "q3.base.game.mover.initializeBinary.use",
            Rc::new(move |driver, slot, other, activator| {
                let result = use_runtime
                    .borrow_mut()
                    .use_binary(driver, slot, other.cloned(), activator.cloned());
                or_panic(result);
            }),
        )?;
        let reached_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().reached.intern(
            "q3.base.game.mover.initializeBinary.reached",
            Rc::new(move |driver, slot| {
                let result = reached_runtime.borrow_mut().reached_binary(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unified from `mirrors_game_state.rs` (hoist: q3 state mirror).
// ---------------------------------------------------------------------------

/// Mover shared-actor body kind (`SharedMoverBody["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedBodyKind {
    /// Player.
    Player,
    /// Movable.
    Movable,
    /// Fixed.
    Fixed,
    /// Attached.
    Attached,
}

/// Shared mover body (`SharedMoverBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct SharedMoverBody {
    /// Actor.
    pub actor: ActorId,
    /// Kind.
    pub kind: SharedBodyKind,
    /// State.
    pub state: Q3BodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Clip mask.
    pub clip_mask: i32,
}

/// Mover actor access (`MoverActorAccess` from `mover.ts`).
pub trait MoverActorAccess {
    /// Native slot for an actor (`native`).
    fn native_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Participant for an actor (`participant`).
    fn participant(&self, actor: &ActorId) -> Participant;
    /// Observe a shared body (`observe`).
    fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody>;
    /// Write origin and ground (`write`).
    fn write(&mut self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>);
    /// Link an actor (`link`).
    fn link_actor(&mut self, actor: &ActorId);
    /// Release an actor (`release`).
    fn release(&mut self, actor: &ActorId);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use qa_core::math::vec3;

    use std::cell::RefCell;

    use crate::q3::base::shared::definitions::{EntityType, Product};
    use std::rc::Rc;

    #[test]
    fn mover_binary_cycle() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Product::Baseq3);
        driver.combat.product = Product::Baseq3;
        let runtime = MoverRuntime::new(900);
        let slot = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[slot].pos1 = vec3(0.0, 0.0, 0.0);
        driver.pool.entities[slot].pos2 = vec3(0.0, 0.0, 100.0);
        let variables = SpawnVariables::new(Vec::new()).unwrap();
        let rt = Rc::new(RefCell::new(runtime));
        MoverRuntime::bind_save_callbacks(&rt, &mut driver).unwrap();
        rt.borrow().initialize_binary(&mut driver, slot, &variables).unwrap();
        assert_eq!(driver.pool.entities[slot].s.e_type, EntityType::EtMover as i32);
        assert!(driver.pool.entities[slot].s.pos.duration >= 1);
        rt.borrow().use_binary(&mut driver, slot, None, None).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::OneToTwo as i32);
        assert_eq!(driver.portals, vec![(slot, true)]);
        rt.borrow()
            .set_state(&mut driver, slot, MoverState::OneToTwo, 1000)
            .unwrap();
        rt.borrow().reached_binary(&mut driver, slot).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::Pos2 as i32);
        assert_eq!(driver.use_targets_calls.len(), 1);
        rt.borrow().return_to_pos1(&mut driver, slot).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::TwoToOne as i32);
        driver.combat.product = Product::Missionpack;
        assert!(rt.borrow().run(&mut driver, slot).is_err());
    }
}
