//! Quake I monster movement builtins.
//!
//! Donor provenance: `src/movement/q1/monsters.ts` (from WinQuake
//! `sv_move.c` and `PF_changeyaw`/`PF_walkmove` in `pr_cmds.c`).

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericOps;

use super::super::types::{MovementError, TraceHit};
use super::common::MovementMath;
use super::types::{
    Q1Trace, Q1_CONTENTS_EMPTY, Q1_CONTENTS_SOLID, Q1_FLAG_FLY, Q1_FLAG_ONGROUND, Q1_FLAG_SWIM, Q1_STEP_HEIGHT,
};

/// Partial-ground flag.
pub const Q1_FLAG_PARTIALGROUND: i32 = 1024;

/// Monster move state snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MonsterMoveState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Entity flags.
    pub flags: i32,
    /// Ground hit.
    pub ground: TraceHit,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Enemy actor.
    pub enemy: Option<ActorId>,
}

/// Monster target snapshot (origin plus absolute bounds).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MonsterTarget {
    /// Origin.
    pub origin: Vec3,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
}

/// Monster movement services: collision, randomness, and actor storage.
pub trait Q1MonsterMoveServices {
    /// Numeric operations.
    fn numeric(&self) -> NumericOps;
    /// Trace with optional box bounds (`None` selects a point trace).
    fn trace(&mut self, actor: &ActorId, start: Vec3, end: Vec3, bounds: Option<Bounds>) -> Q1Trace;
    /// Q1-translated point contents.
    fn point_contents(&mut self, actor: &ActorId, point: Vec3) -> i32;
    /// Next random integer.
    fn next_random(&mut self) -> i32;
    /// Read a monster snapshot.
    fn read(&mut self, actor: &ActorId) -> Option<Q1MonsterMoveState>;
    /// Read a target snapshot.
    fn read_target(&mut self, actor: &ActorId) -> Option<Q1MonsterTarget>;
    /// Write a monster snapshot.
    fn write(&mut self, actor: &OwnedActor, state: Q1MonsterMoveState);
    /// Link an actor; source linking may run triggers, teleport, or remove.
    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool);
}

/// Movement builtins sharing the simulation's actor, collision, and random
/// owners.
pub struct Q1MonsterMovement<S> {
    services: S,
    math: MovementMath,
}

/// Build monster movement over services.
pub fn create_q1_monster_movement<S: Q1MonsterMoveServices>(services: S) -> Q1MonsterMovement<S> {
    Q1MonsterMovement::new(services)
}

impl<S: Q1MonsterMoveServices> Q1MonsterMovement<S> {
    /// Build monster movement over services.
    pub fn new(services: S) -> Self {
        let math = MovementMath::new(services.numeric());
        Self { services, math }
    }

    fn angle_mod(&self, angle: f64) -> f64 {
        let n = self.services.numeric();
        n.mul(
            360.0 / 65536.0,
            f64::from(n.to_int32(n.mul(angle, 65536.0 / 360.0)).unwrap_or(0) & 65535),
        )
    }

    /// Turn toward the ideal yaw at yaw speed.
    pub fn change_yaw(&mut self, actor: &OwnedActor) {
        let Some(state) = self.services.read(actor.id()) else {
            return;
        };
        let n = self.services.numeric();
        let current = self.angle_mod(f64::from(state.angles.y));
        let ideal = state.ideal_yaw;
        if current == ideal {
            return;
        }
        let mut step = n.sub(ideal, current);
        if ideal > current {
            if step >= 180.0 {
                step = n.sub(step, 360.0);
            }
        } else if step <= -180.0 {
            step = n.add(step, 360.0);
        }
        step = if step > 0.0 {
            step.min(state.yaw_speed)
        } else {
            step.max(-state.yaw_speed)
        };
        let angles = self.math.vec(
            f64::from(state.angles.x),
            self.angle_mod(n.add(current, step)),
            f64::from(state.angles.z),
        );
        self.services.write(actor, Q1MonsterMoveState { angles, ..state });
    }

    /// Check that all corners rest over ground.
    pub fn check_bottom(&mut self, actor: &ActorId) -> bool {
        let Some(state) = self.services.read(actor) else {
            return false;
        };
        let minimum = self.math.add(state.origin, state.bounds.min);
        let maximum = self.math.add(state.origin, state.bounds.max);
        let n = self.services.numeric();
        let mut easy = true;
        'corners: for x in [minimum.x, maximum.x] {
            for y in [minimum.y, maximum.y] {
                let point = self
                    .math
                    .vec(f64::from(x), f64::from(y), n.sub(f64::from(minimum.z), 1.0));
                if self.services.point_contents(actor, point) != Q1_CONTENTS_SOLID {
                    easy = false;
                    break 'corners;
                }
            }
        }
        if easy {
            return true;
        }
        let middle = self.math.vec(
            n.mul(n.add(f64::from(minimum.x), f64::from(maximum.x)), 0.5),
            n.mul(n.add(f64::from(minimum.y), f64::from(maximum.y)), 0.5),
            f64::from(minimum.z),
        );
        let end_z = n.sub(f64::from(minimum.z), 2.0 * Q1_STEP_HEIGHT);
        let trace = self.services.trace(
            actor,
            middle,
            self.math.vec(f64::from(middle.x), f64::from(middle.y), end_z),
            None,
        );
        if trace.fraction == 1.0 {
            return false;
        }
        let middle_height = trace.end.z;
        for x in [minimum.x, maximum.x] {
            for y in [minimum.y, maximum.y] {
                let corner = self.services.trace(
                    actor,
                    self.math.vec(f64::from(x), f64::from(y), f64::from(minimum.z)),
                    self.math.vec(f64::from(x), f64::from(y), end_z),
                    None,
                );
                if corner.fraction == 1.0 || n.sub(f64::from(middle_height), f64::from(corner.end.z)) > Q1_STEP_HEIGHT {
                    return false;
                }
            }
        }
        true
    }

    /// Step-move a monster, with step-up and bottom checks for walkers.
    pub fn move_step(&mut self, actor: &OwnedActor, step: Vec3, relink: bool) -> Result<bool, MovementError> {
        let Some(mut state) = self.services.read(actor.id()) else {
            return Ok(false);
        };
        let original = state.origin;
        if state.flags & (Q1_FLAG_SWIM | Q1_FLAG_FLY) != 0 {
            for attempt in 0..2 {
                let mut destination = self.math.add(state.origin, step);
                let enemy = state.enemy.as_ref().and_then(|enemy| self.services.read_target(enemy));
                if attempt == 0 {
                    if let Some(enemy) = &enemy {
                        let n = self.services.numeric();
                        let dz = n.sub(f64::from(state.origin.z), f64::from(enemy.origin.z));
                        if dz > 40.0 {
                            destination = self.math.vec(
                                f64::from(destination.x),
                                f64::from(destination.y),
                                n.sub(f64::from(destination.z), 8.0),
                            );
                        }
                        if dz < 30.0 {
                            destination = self.math.vec(
                                f64::from(destination.x),
                                f64::from(destination.y),
                                n.add(f64::from(destination.z), 8.0),
                            );
                        }
                    }
                }
                let trace = self
                    .services
                    .trace(actor.id(), state.origin, destination, Some(state.bounds));
                if trace.fraction == 1.0 {
                    if state.flags & Q1_FLAG_SWIM != 0
                        && self.services.point_contents(actor.id(), trace.end) == Q1_CONTENTS_EMPTY
                    {
                        return Ok(false);
                    }
                    self.services.write(
                        actor,
                        Q1MonsterMoveState {
                            origin: trace.end,
                            ..state
                        },
                    );
                    if relink {
                        self.services.link(actor, true);
                    }
                    return Ok(true);
                }
                if enemy.is_none() {
                    break;
                }
            }
            return Ok(false);
        }
        let n = self.services.numeric();
        let mut destination = self.math.add(state.origin, step);
        destination = self.math.vec(
            f64::from(destination.x),
            f64::from(destination.y),
            n.add(f64::from(destination.z), Q1_STEP_HEIGHT),
        );
        let end = self.math.vec(
            f64::from(destination.x),
            f64::from(destination.y),
            n.sub(f64::from(destination.z), Q1_STEP_HEIGHT * 2.0),
        );
        let mut trace = self.services.trace(actor.id(), destination, end, Some(state.bounds));
        if trace.all_solid {
            return Ok(false);
        }
        if trace.start_solid {
            destination = self.math.vec(
                f64::from(destination.x),
                f64::from(destination.y),
                n.sub(f64::from(destination.z), Q1_STEP_HEIGHT),
            );
            trace = self.services.trace(actor.id(), destination, end, Some(state.bounds));
            if trace.all_solid || trace.start_solid {
                return Ok(false);
            }
        }
        if trace.fraction == 1.0 {
            if state.flags & Q1_FLAG_PARTIALGROUND == 0 {
                return Ok(false);
            }
            let origin = self.math.add(state.origin, step);
            self.services.write(actor, Q1MonsterMoveState { origin, ..state });
            if relink {
                self.services.link(actor, true);
            }
            if let Some(state) = self.services.read(actor.id()) {
                self.services.write(
                    actor,
                    Q1MonsterMoveState {
                        flags: state.flags & !Q1_FLAG_ONGROUND,
                        ..state
                    },
                );
            }
            return Ok(true);
        }
        state = Q1MonsterMoveState {
            origin: trace.end,
            ..state
        };
        self.services.write(actor, state.clone());
        if !self.check_bottom(actor.id()) {
            if state.flags & Q1_FLAG_PARTIALGROUND != 0 {
                if relink {
                    self.services.link(actor, true);
                }
                return Ok(true);
            }
            self.services.write(
                actor,
                Q1MonsterMoveState {
                    origin: original,
                    ..state
                },
            );
            return Ok(false);
        }
        if matches!(trace.hit, TraceHit::None) {
            return Err(MovementError::Contract(
                "Quake monster step landed without a ground hit",
            ));
        }
        self.services.write(
            actor,
            Q1MonsterMoveState {
                flags: state.flags & !Q1_FLAG_PARTIALGROUND,
                ground: trace.hit,
                ..state
            },
        );
        if relink {
            self.services.link(actor, true);
        }
        Ok(true)
    }

    /// Step in a yaw direction, rejecting turns over 45 degrees.
    pub fn step_direction(&mut self, actor: &OwnedActor, yaw: f64, distance: f64) -> Result<bool, MovementError> {
        let Some(state) = self.services.read(actor.id()) else {
            return Ok(false);
        };
        let n = self.services.numeric();
        self.services.write(
            actor,
            Q1MonsterMoveState {
                ideal_yaw: f64::from(n.store(yaw)),
                ..state
            },
        );
        self.change_yaw(actor);
        let Some(state) = self.services.read(actor.id()) else {
            return Ok(false);
        };
        let radians = n.div(n.mul(n.mul(yaw, std::f64::consts::PI), 2.0), 360.0);
        let step = self
            .math
            .vec(n.mul(radians.cos(), distance), n.mul(radians.sin(), distance), 0.0);
        let original = state.origin;
        let moved = self.move_step(actor, step, false)?;
        let Some(state) = self.services.read(actor.id()) else {
            return Ok(moved);
        };
        if moved {
            let delta = n.sub(f64::from(state.angles.y), state.ideal_yaw);
            if delta > 45.0 && delta < 315.0 {
                self.services.write(
                    actor,
                    Q1MonsterMoveState {
                        origin: original,
                        ..state
                    },
                );
            }
        }
        self.services.link(actor, true);
        Ok(moved)
    }

    /// Walk-move in a yaw direction when grounded or flying.
    pub fn walk_move(&mut self, actor: &OwnedActor, yaw: f64, distance: f64) -> Result<bool, MovementError> {
        let Some(state) = self.services.read(actor.id()) else {
            return Ok(false);
        };
        if state.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) == 0 {
            return Ok(false);
        }
        let n = self.services.numeric();
        let radians = n.div(n.mul(n.mul(yaw, std::f64::consts::PI), 2.0), 360.0);
        let step = self
            .math
            .vec(n.mul(radians.cos(), distance), n.mul(radians.sin(), distance), 0.0);
        self.move_step(actor, step, true)
    }

    /// Whether two actors' absolute bounds overlap within a distance.
    pub fn close_enough(&mut self, actor: &ActorId, goal: &ActorId, distance: f64) -> bool {
        let (Some(me), Some(target)) = (self.services.read(actor), self.services.read_target(goal)) else {
            return false;
        };
        let n = self.services.numeric();
        let a = me.absolute_bounds;
        let b = target.absolute_bounds;
        f64::from(b.min.x) <= n.add(f64::from(a.max.x), distance)
            && f64::from(b.max.x) >= n.sub(f64::from(a.min.x), distance)
            && f64::from(b.min.y) <= n.add(f64::from(a.max.y), distance)
            && f64::from(b.max.y) >= n.sub(f64::from(a.min.y), distance)
            && f64::from(b.min.z) <= n.add(f64::from(a.max.z), distance)
            && f64::from(b.max.z) >= n.sub(f64::from(a.min.z), distance)
    }

    /// Pick a chase direction toward a goal.
    pub fn new_chase_direction(
        &mut self,
        actor: &OwnedActor,
        goal: &ActorId,
        distance: f64,
    ) -> Result<(), MovementError> {
        let (Some(state), Some(enemy)) = (self.services.read(actor.id()), self.services.read_target(goal)) else {
            return Ok(());
        };
        let n = self.services.numeric();
        let old_direction =
            self.angle_mod(n.mul(f64::from(n.to_int32(n.div(state.ideal_yaw, 45.0)).unwrap_or(0)), 45.0));
        let turnaround = self.angle_mod(n.sub(old_direction, 180.0));
        let dx = n.sub(f64::from(enemy.origin.x), f64::from(state.origin.x));
        let dy = n.sub(f64::from(enemy.origin.y), f64::from(state.origin.y));
        let mut first = if dx > 10.0 {
            0.0
        } else if dx < -10.0 {
            180.0
        } else {
            -1.0
        };
        let mut second = if dy < -10.0 {
            270.0
        } else if dy > 10.0 {
            90.0
        } else {
            -1.0
        };
        if first != -1.0 && second != -1.0 {
            // The southwest constant is 215 in both released source and donor.
            let diagonal = if first == 0.0 {
                if second == 90.0 {
                    45.0
                } else {
                    315.0
                }
            } else if second == 90.0 {
                135.0
            } else {
                215.0
            };
            if diagonal != turnaround && self.try_step(actor, diagonal, distance)? {
                return Ok(());
            }
        }
        if self.services.next_random() & 3 & 1 != 0 || dy.abs() > dx.abs() {
            std::mem::swap(&mut first, &mut second);
        }
        if first != -1.0 && first != turnaround && self.try_step(actor, first, distance)? {
            return Ok(());
        }
        if second != -1.0 && second != turnaround && self.try_step(actor, second, distance)? {
            return Ok(());
        }
        if old_direction != -1.0 && self.try_step(actor, old_direction, distance)? {
            return Ok(());
        }
        if self.services.next_random() & 1 != 0 {
            let mut direction = 0.0;
            while direction <= 315.0 {
                if direction != turnaround && self.try_step(actor, direction, distance)? {
                    return Ok(());
                }
                direction += 45.0;
            }
        } else {
            let mut direction = 315.0;
            while direction >= 0.0 {
                if direction != turnaround && self.try_step(actor, direction, distance)? {
                    return Ok(());
                }
                direction -= 45.0;
            }
        }
        if turnaround != -1.0 && self.try_step(actor, turnaround, distance)? {
            return Ok(());
        }
        if let Some(current) = self.services.read(actor.id()) {
            self.services.write(
                actor,
                Q1MonsterMoveState {
                    ideal_yaw: old_direction,
                    ..current
                },
            );
            if !self.check_bottom(actor.id()) {
                if let Some(after) = self.services.read(actor.id()) {
                    self.services.write(
                        actor,
                        Q1MonsterMoveState {
                            flags: after.flags | Q1_FLAG_PARTIALGROUND,
                            ..after
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn try_step(&mut self, actor: &OwnedActor, direction: f64, distance: f64) -> Result<bool, MovementError> {
        if self.services.read(actor.id()).is_none() {
            return Ok(true);
        }
        self.step_direction(actor, direction, distance)
    }

    /// Move toward a goal, repicking the chase direction on failure.
    pub fn move_to_goal(
        &mut self,
        actor: &OwnedActor,
        goal: &ActorId,
        distance: f64,
        contact: bool,
    ) -> Result<(), MovementError> {
        let Some(state) = self.services.read(actor.id()) else {
            return Ok(());
        };
        if state.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) == 0 {
            return Ok(());
        }
        if !contact && state.enemy.is_some() && self.close_enough(actor.id(), goal, distance) {
            return Ok(());
        }
        if (self.services.next_random() & 3 == 1 || !self.step_direction(actor, state.ideal_yaw, distance)?)
            && self.services.read(actor.id()).is_some()
        {
            self.new_chase_direction(actor, goal, distance)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};

    struct Harness {
        ops: NumericOps,
        state: Option<Q1MonsterMoveState>,
        target: Option<Q1MonsterTarget>,
        random: Vec<i32>,
        solid_corners: bool,
        open: bool,
    }

    impl Q1MonsterMoveServices for Harness {
        fn numeric(&self) -> NumericOps {
            self.ops
        }
        fn trace(&mut self, _actor: &ActorId, start: Vec3, end: Vec3, _bounds: Option<Bounds>) -> Q1Trace {
            let fraction = if self.open { 1.0 } else { 0.5 };
            Q1Trace {
                fraction,
                end: if self.open { end } else { start },
                start_solid: false,
                all_solid: false,
                contact: super::super::super::types::TraceContact::None,
                hit: TraceHit::World { model: 0 },
                in_open: true,
                in_water: false,
                source_plane: qa_core::math::Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            }
        }
        fn point_contents(&mut self, _actor: &ActorId, _point: Vec3) -> i32 {
            if self.solid_corners {
                Q1_CONTENTS_SOLID
            } else {
                Q1_CONTENTS_EMPTY
            }
        }
        fn next_random(&mut self) -> i32 {
            self.random.pop().unwrap_or(0)
        }
        fn read(&mut self, _actor: &ActorId) -> Option<Q1MonsterMoveState> {
            self.state.clone()
        }
        fn read_target(&mut self, _actor: &ActorId) -> Option<Q1MonsterTarget> {
            self.target.clone()
        }
        fn write(&mut self, _actor: &OwnedActor, state: Q1MonsterMoveState) {
            self.state = Some(state);
        }
        fn link(&mut self, _actor: &OwnedActor, _touch_triggers: bool) {}
    }

    fn monster() -> Q1MonsterMoveState {
        Q1MonsterMoveState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            absolute_bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            flags: Q1_FLAG_ONGROUND,
            ground: TraceHit::World { model: 0 },
            ideal_yaw: 90.0,
            yaw_speed: 45.0,
            enemy: None,
        }
    }

    fn harness() -> (IdentityOwner, Harness) {
        let owner = IdentityOwner::create("q1-monsters").unwrap();
        let harness = Harness {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
            state: Some(monster()),
            target: None,
            random: vec![0],
            solid_corners: true,
            open: true,
        };
        (owner, harness)
    }

    #[test]
    fn yaw_turns_at_yaw_speed() {
        let (owner, harness) = harness();
        let id = owner.actor(1, 0);
        let actor = owner
            .owned_actor(&id, qa_core::identity::ProviderId::new("q1", "test"))
            .unwrap();
        let mut movement = Q1MonsterMovement::new(harness);
        movement.change_yaw(&actor);
        assert_eq!(movement.services.state.unwrap().angles.y, 45.0);
    }

    #[test]
    fn solid_corners_pass_bottom_check() {
        let (owner, harness) = harness();
        let id = owner.actor(1, 0);
        let mut movement = Q1MonsterMovement::new(harness);
        assert!(movement.check_bottom(&id));
    }

    #[test]
    fn open_step_moves_walker() {
        let (owner, harness) = harness();
        let id = owner.actor(1, 0);
        let actor = owner
            .owned_actor(&id, qa_core::identity::ProviderId::new("q1", "test"))
            .unwrap();
        let mut movement = Q1MonsterMovement::new(harness);
        // Open middle trace with fraction 1 fails the bottom middle probe, so
        // force the easy corner path only.
        movement.services.open = false;
        let moved = movement.move_step(&actor, vec3(8.0, 0.0, 0.0), true).unwrap();
        assert!(moved);
    }

    #[test]
    fn close_enough_compares_absolute_bounds() {
        let (owner, mut harness) = harness();
        let id = owner.actor(1, 0);
        let goal = owner.actor(2, 0);
        harness.target = Some(Q1MonsterTarget {
            origin: vec3(100.0, 0.0, 0.0),
            absolute_bounds: Bounds {
                min: vec3(84.0, -16.0, -24.0),
                max: vec3(116.0, 16.0, 32.0),
            },
        });
        let mut movement = Q1MonsterMovement::new(harness);
        assert!(!movement.close_enough(&id, &goal, 10.0));
        assert!(movement.close_enough(&id, &goal, 100.0));
    }

    #[test]
    fn walk_move_rejects_airborne() {
        let (owner, mut harness) = harness();
        let id = owner.actor(1, 0);
        let actor = owner
            .owned_actor(&id, qa_core::identity::ProviderId::new("q1", "test"))
            .unwrap();
        harness.state.as_mut().unwrap().flags = 0;
        let mut movement = Q1MonsterMovement::new(harness);
        assert!(!movement.walk_move(&actor, 0.0, 8.0).unwrap());
    }
}
