//! Quake I pusher transactions.
//!
//! Donor provenance: `src/movement/q1/pusher.ts` (`SV_PushMove` and the
//! separately selected rotating-pusher extension).

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericOps;

use super::common::MovementMath;
use super::super::types::{MovementError, TraceHit};
use super::types::{Q1_FLAG_ONGROUND, Q1_MOVE_NOCLIP, Q1_MOVE_NONE, Q1_MOVE_PUSH, Q1_MOVE_WALK,
    Q1PhysicsEntity, Q1PusherResult, Q1PusherServices, Q1PusherStatus, Q1Solid};

fn overlap(a: &Bounds, b: &Bounds) -> bool {
    a.min.x < b.max.x
        && a.min.y < b.max.y
        && a.min.z < b.max.z
        && a.max.x > b.min.x
        && a.max.y > b.min.y
        && a.max.z > b.min.z
}

fn rider(entity: &Q1PhysicsEntity, pusher: &ActorId) -> bool {
    entity.state.flags & Q1_FLAG_ONGROUND != 0
        && matches!(&entity.state.ground, TraceHit::Actor { actor } if actor == pusher)
}

/// Pusher movement selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1PusherMovement {
    /// Ordinary translational NetQuake push.
    Translate,
    /// Rotating-pusher extension.
    Rotate,
}

/// Velocity-driven pusher input, mirroring donor `Q1PusherInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PusherInput {
    /// Pusher actor.
    pub actor: ActorId,
    /// Elapsed time in seconds.
    pub elapsed_seconds: f64,
    /// Movement selector.
    pub movement: Q1PusherMovement,
}

/// Displacement pusher input, mirroring donor `Q1PushInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PushInput {
    /// Pusher actor.
    pub actor: ActorId,
    /// Linear displacement.
    pub displacement: Vec3,
    /// Angular displacement.
    pub angular_displacement: Vec3,
    /// Elapsed time in seconds.
    pub elapsed_seconds: f64,
}

fn removed(actor: &ActorId) -> Q1PusherResult {
    Q1PusherResult {
        actor: actor.clone(),
        status: Q1PusherStatus::ActorRemoved,
        moved: Vec::new(),
    }
}

/// Velocity-driven source entry; displacement callers share the same
/// transaction.
pub fn move_q1_pusher<S: Q1PusherServices>(
    input: &Q1PusherInput,
    services: &mut S,
) -> Result<Q1PusherResult, MovementError> {
    let Some(pusher) = services.read(&input.actor) else {
        return Ok(removed(&input.actor));
    };
    let math = MovementMath::new(services.numeric());
    let zero = math.vec(0.0, 0.0, 0.0);
    let source = if input.movement == Q1PusherMovement::Rotate {
        pusher.state.angular_velocity
    } else {
        pusher.state.velocity
    };
    let delta = math.scale(source, input.elapsed_seconds);
    push_q1_pusher(
        &Q1PushInput {
            actor: input.actor.clone(),
            elapsed_seconds: input.elapsed_seconds,
            displacement: if input.movement == Q1PusherMovement::Translate {
                delta
            } else {
                zero
            },
            angular_displacement: if input.movement == Q1PusherMovement::Rotate {
                delta
            } else {
                zero
            },
        },
        services,
    )
}

/// A single source pusher transaction. The shared scheduler chooses its
/// traversal.
pub fn push_q1_pusher<S: Q1PusherServices>(
    input: &Q1PushInput,
    services: &mut S,
) -> Result<Q1PusherResult, MovementError> {
    let Some(mut pusher) = services.read(&input.actor) else {
        return Ok(removed(&input.actor));
    };
    if !input.elapsed_seconds.is_finite() || input.elapsed_seconds < 0.0 {
        return Err(MovementError::Range("Invalid pusher interval"));
    }
    let math = MovementMath::new(services.numeric());
    let n: NumericOps = math.n;
    let original_origin = pusher.state.origin;
    let original_angles = pusher.state.angles;
    let original_bounds = pusher.absolute_bounds;
    let delta = input.displacement;
    let angular = input.angular_displacement;
    let rotating = angular.x != 0.0 || angular.y != 0.0 || angular.z != 0.0;
    if delta.x == 0.0 && delta.y == 0.0 && delta.z == 0.0 && !rotating {
        pusher.local_time_seconds = f64::from(n.store(n.add(pusher.local_time_seconds, input.elapsed_seconds)));
        services.write(pusher);
        return Ok(Q1PusherResult {
            actor: input.actor.clone(),
            status: Q1PusherStatus::Moved,
            moved: Vec::new(),
        });
    }
    pusher.state.origin = math.add(pusher.state.origin, delta);
    pusher.state.angles = math.add(pusher.state.angles, angular);
    pusher.local_time_seconds = f64::from(n.store(n.add(pusher.local_time_seconds, input.elapsed_seconds)));
    services.write(pusher.clone());
    services.link(&pusher.actor, false);
    let mut pushed: Vec<(ActorId, Vec3)> = Vec::new();
    let axes = math.angles(math.scale(angular, -1.0));
    let Some(linked_pusher) = services.read(&input.actor) else {
        return Ok(removed(&input.actor));
    };
    let pusher_bounds = if !rotating {
        Bounds {
            min: math.add(original_bounds.min, delta),
            max: math.add(original_bounds.max, delta),
        }
    } else {
        linked_pusher.absolute_bounds
    };
    for actor in services.candidates() {
        let Some(mut entity) = services.read(&actor) else {
            continue;
        };
        if actor == input.actor {
            continue;
        }
        if entity.state.move_type == Q1_MOVE_PUSH
            || entity.state.move_type == Q1_MOVE_NONE
            || entity.state.move_type == Q1_MOVE_NOCLIP
        {
            continue;
        }
        if !rider(&entity, &input.actor) {
            if !overlap(&entity.absolute_bounds, &pusher_bounds) {
                continue;
            }
            if matches!(services.test_position(&entity), TraceHit::None) {
                continue;
            }
        }
        if entity.state.move_type != Q1_MOVE_WALK {
            entity.state.flags &= !Q1_FLAG_ONGROUND;
        }
        let original = entity.state.origin;
        pushed.push((actor.clone(), original));
        let mut displacement = delta;
        if rotating {
            let offset = math.sub(math.add(entity.state.origin, delta), pusher.state.origin);
            let rotated = math.vec(
                math.dot(offset, axes.forward),
                -math.dot(offset, axes.right),
                math.dot(offset, axes.up),
            );
            displacement = math.add(delta, math.sub(rotated, offset));
        }
        services.collision_enabled(&pusher.actor, false);
        let pushed_entity = services.push(&entity, displacement).0;
        // A synchronous trigger may remove the pusher. Never relink a stale actor.
        if services.read(&input.actor).is_some() {
            services.collision_enabled(&pusher.actor, true);
        }
        let Some(live_pusher) = services.read(&input.actor) else {
            return Ok(Q1PusherResult {
                actor: input.actor.clone(),
                status: Q1PusherStatus::ActorRemoved,
                moved: pushed.iter().map(|(actor, _)| actor.clone()).collect(),
            });
        };
        pusher = live_pusher;
        let Some(entity) = pushed_entity else {
            continue;
        };
        if matches!(services.test_position(&entity), TraceHit::None) {
            if rotating {
                let angles = math.add(entity.state.angles, angular);
                let mut entity = entity;
                entity.state.angles = angles;
                services.write(entity);
            }
            continue;
        }
        if entity.bounds.min.x == entity.bounds.max.x {
            continue;
        }
        if entity.solid == Q1Solid::Not
            || entity.solid == Q1Solid::Trigger
            || entity.solid == Q1Solid::Corpse
        {
            let minimum = math.vec(0.0, 0.0, f64::from(entity.bounds.min.z));
            let mut entity = entity;
            entity.bounds = Bounds { min: minimum, max: minimum };
            services.write(entity);
            continue;
        }
        let mut entity = entity;
        entity.state.origin = original;
        let failed_actor = actor.clone();
        services.write(entity.clone());
        services.link(&entity.actor, true);
        let Some(current_pusher) = services.read(&input.actor) else {
            return Ok(Q1PusherResult {
                actor: input.actor.clone(),
                status: Q1PusherStatus::ActorRemoved,
                moved: pushed.iter().map(|(actor, _)| actor.clone()).collect(),
            });
        };
        let restored_time =
            f64::from(n.store(n.sub(current_pusher.local_time_seconds, input.elapsed_seconds)));
        let mut current_pusher = current_pusher;
        current_pusher.state.origin = original_origin;
        current_pusher.state.angles = original_angles;
        current_pusher.local_time_seconds = restored_time;
        services.write(current_pusher.clone());
        services.link(&current_pusher.actor, false);
        services.blocked(&current_pusher.actor, &failed_actor);
        // Source blocked() runs before rollback, so preserve its damage,
        // removals and state changes while restoring only the positions
        // owned by this transaction.
        for (moved_actor, moved_origin) in &pushed {
            let Some(current) = services.read(moved_actor) else {
                continue;
            };
            let mut current = current;
            current.state.origin = *moved_origin;
            if rotating {
                current.state.angles = math.sub(current.state.angles, angular);
            }
            services.write(current.clone());
            services.link(&current.actor, false);
        }
        let status = if services.read(&input.actor).is_none() {
            Q1PusherStatus::ActorRemoved
        } else {
            Q1PusherStatus::Blocked
        };
        return Ok(Q1PusherResult {
            actor: input.actor.clone(),
            status,
            moved: pushed.iter().map(|(actor, _)| actor.clone()).collect(),
        });
    }
    Ok(Q1PusherResult {
        actor: input.actor.clone(),
        status: Q1PusherStatus::Moved,
        moved: pushed.iter().map(|(actor, _)| actor.clone()).collect(),
    })
}

/// Think timing uses local pusher time, including blocked moves that do not
/// advance.
pub fn step_q1_pusher<S: Q1PusherServices>(
    input: &Q1PusherInput,
    services: &mut S,
) -> Result<Q1PusherResult, MovementError> {
    let Some(pusher) = services.read(&input.actor) else {
        return Ok(removed(&input.actor));
    };
    let n = services.numeric();
    let old_time = pusher.local_time_seconds;
    let think_time = pusher.next_think_seconds;
    let move_time = if think_time < n.add(old_time, input.elapsed_seconds) {
        n.sub(think_time, old_time).max(0.0)
    } else {
        input.elapsed_seconds
    };
    let result = if move_time == 0.0 {
        Q1PusherResult {
            actor: input.actor.clone(),
            status: Q1PusherStatus::Moved,
            moved: Vec::new(),
        }
    } else {
        move_q1_pusher(&Q1PusherInput { elapsed_seconds: move_time, ..input.clone() }, services)?
    };
    let Some(current) = services.read(&input.actor) else {
        return Ok(Q1PusherResult {
            status: Q1PusherStatus::ActorRemoved,
            ..result
        });
    };
    if think_time > old_time && think_time <= current.local_time_seconds {
        let mut current = current;
        current.next_think_seconds = 0.0;
        let actor: OwnedActor = current.actor.clone();
        services.write(current);
        services.think(&actor);
    }
    if services.read(&input.actor).is_none() {
        return Ok(Q1PusherResult {
            status: Q1PusherStatus::ActorRemoved,
            ..result
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::vec3;
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use std::collections::HashMap;

    use super::super::types::{Q1MovementState, Q1Solid};

    struct Harness {
        ops: NumericOps,
        entities: HashMap<(u32, u32), Q1PhysicsEntity>,
        order: Vec<ActorId>,
        blocked: Vec<ActorId>,
        thinks: usize,
        solid_positions: bool,
    }

    impl Q1PusherServices for Harness {
        fn numeric(&self) -> NumericOps {
            self.ops
        }
        fn read(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
            self.entities.get(&(actor.slot(), actor.generation())).cloned()
        }
        fn candidates(&mut self) -> Vec<ActorId> {
            self.order.clone()
        }
        fn write(&mut self, entity: Q1PhysicsEntity) {
            let id = entity.actor.id().clone();
            self.entities.insert((id.slot(), id.generation()), entity);
        }
        fn link(&mut self, _actor: &OwnedActor, _touch_triggers: bool) {}
        fn collision_enabled(&mut self, _actor: &OwnedActor, _enabled: bool) {}
        fn test_position(&mut self, entity: &Q1PhysicsEntity) -> TraceHit {
            if self.solid_positions {
                TraceHit::World { model: 0 }
            } else {
                let _ = entity;
                TraceHit::None
            }
        }
        fn push(
            &mut self,
            entity: &Q1PhysicsEntity,
            displacement: Vec3,
        ) -> (Option<Q1PhysicsEntity>, super::super::types::Q1Trace) {
            let mut entity = entity.clone();
            entity.state.origin = Vec3 {
                x: entity.state.origin.x + displacement.x,
                y: entity.state.origin.y + displacement.y,
                z: entity.state.origin.z + displacement.z,
            };
            let trace = super::super::types::Q1Trace {
                fraction: 1.0,
                end: entity.state.origin,
                start_solid: false,
                all_solid: false,
                contact: super::super::super::types::TraceContact::None,
                hit: TraceHit::None,
                in_open: true,
                in_water: false,
                source_plane: qa_core::math::Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            };
            (Some(entity), trace)
        }
        fn blocked(&mut self, _pusher: &OwnedActor, obstacle: &ActorId) {
            self.blocked.push(obstacle.clone());
        }
        fn think(&mut self, _pusher: &OwnedActor) {
            self.thinks += 1;
        }
    }

    fn entity(owner: &IdentityOwner, slot: u32, move_type: i32, origin: Vec3) -> Q1PhysicsEntity {
        let id = owner.actor(slot, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        Q1PhysicsEntity {
            actor,
            state: Q1MovementState {
                origin,
                velocity: vec3(10.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                old_origin: origin,
                angular_velocity: vec3(0.0, 0.0, 0.0),
                view_angles: vec3(0.0, 0.0, 0.0),
                punch_angles: vec3(0.0, 0.0, 0.0),
                move_type,
                flags: 0,
                ground: TraceHit::None,
                water_level: 0,
                water_type: -1,
                teleport_time_seconds: 0.0,
                water_jump_direction: vec3(0.0, 0.0, 0.0),
                ideal_pitch: 0.0,
                fix_angle: false,
                health: 100.0,
            },
            bounds: Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
            absolute_bounds: Bounds {
                min: vec3(origin.x - 8.0, origin.y - 8.0, origin.z - 8.0),
                max: vec3(origin.x + 8.0, origin.y + 8.0, origin.z + 8.0),
            },
            solid: Q1Solid::Bsp,
            local_time_seconds: 0.0,
            next_think_seconds: 0.0,
        }
    }

    fn harness() -> (IdentityOwner, Harness) {
        let owner = IdentityOwner::create("q1-pusher").unwrap();
        let pusher = entity(&owner, 1, Q1_MOVE_PUSH, vec3(0.0, 0.0, 0.0));
        let rider = entity(&owner, 2, Q1_MOVE_WALK, vec3(100.0, 0.0, 0.0));
        let mut entities = HashMap::new();
        entities.insert((1, 0), pusher);
        entities.insert((2, 0), rider);
        let order = vec![owner.actor(1, 0), owner.actor(2, 0)];
        (
            owner,
            Harness {
                ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
                entities,
                order,
                blocked: Vec::new(),
                thinks: 0,
                solid_positions: false,
            },
        )
    }

    #[test]
    fn idle_pusher_only_advances_time() {
        let (owner, mut harness) = harness();
        harness.entities.get_mut(&(1, 0)).unwrap().state.velocity = vec3(0.0, 0.0, 0.0);
        let result = move_q1_pusher(
            &Q1PusherInput {
                actor: owner.actor(1, 0),
                elapsed_seconds: 0.1,
                movement: Q1PusherMovement::Translate,
            },
            &mut harness,
        )
        .unwrap();
        assert_eq!(result.status, Q1PusherStatus::Moved);
        assert!(result.moved.is_empty());
        assert_eq!(harness.entities[&(1, 0)].local_time_seconds, f64::from(0.1f32));
    }

    #[test]
    fn moving_pusher_advances_origin() {
        let (owner, mut harness) = harness();
        let result = move_q1_pusher(
            &Q1PusherInput {
                actor: owner.actor(1, 0),
                elapsed_seconds: 0.1,
                movement: Q1PusherMovement::Translate,
            },
            &mut harness,
        )
        .unwrap();
        assert_eq!(result.status, Q1PusherStatus::Moved);
        assert_eq!(harness.entities[&(1, 0)].state.origin.x, 1.0);
    }

    #[test]
    fn blocked_pusher_restores_positions() {
        let (owner, mut harness) = harness();
        harness.solid_positions = true;
        // Overlap the rider with the pusher's swept bounds.
        harness.entities.get_mut(&(2, 0)).unwrap().absolute_bounds = Bounds {
            min: vec3(-4.0, -4.0, -4.0),
            max: vec3(4.0, 4.0, 4.0),
        };
        harness.entities.get_mut(&(2, 0)).unwrap().state.flags = Q1_FLAG_ONGROUND;
        harness.entities.get_mut(&(2, 0)).unwrap().state.ground = TraceHit::Actor {
            actor: owner.actor(1, 0),
        };
        let result = move_q1_pusher(
            &Q1PusherInput {
                actor: owner.actor(1, 0),
                elapsed_seconds: 0.1,
                movement: Q1PusherMovement::Translate,
            },
            &mut harness,
        )
        .unwrap();
        assert_eq!(result.status, Q1PusherStatus::Blocked);
        assert_eq!(harness.entities[&(1, 0)].state.origin.x, 0.0);
        assert_eq!(harness.blocked, vec![owner.actor(2, 0)]);
    }

    #[test]
    fn invalid_interval_is_a_range_error() {
        let (owner, mut harness) = harness();
        let result = push_q1_pusher(
            &Q1PushInput {
                actor: owner.actor(1, 0),
                displacement: vec3(1.0, 0.0, 0.0),
                angular_displacement: vec3(0.0, 0.0, 0.0),
                elapsed_seconds: -1.0,
            },
            &mut harness,
        );
        assert_eq!(result, Err(MovementError::Range("Invalid pusher interval")));
    }

    #[test]
    fn think_fires_when_time_arrives() {
        let (owner, mut harness) = harness();
        harness.entities.get_mut(&(1, 0)).unwrap().next_think_seconds = 0.05;
        let result = step_q1_pusher(
            &Q1PusherInput {
                actor: owner.actor(1, 0),
                elapsed_seconds: 0.1,
                movement: Q1PusherMovement::Translate,
            },
            &mut harness,
        )
        .unwrap();
        assert_eq!(result.status, Q1PusherStatus::Moved);
        assert_eq!(harness.thinks, 1);
        assert_eq!(harness.entities[&(1, 0)].next_think_seconds, 0.0);
    }
}
