//! `PF_walkmove` monster movement and `SV_TouchLinks` trigger dispatch.
//!
//! Ported from donor `src/compat/qc/movement-host.ts`
//! (`createQcMovementBindings`, `createQcTouchCallback`).
//!
//! Local mirrors: [`MonsterScene`] mirrors the `trace`/`pointContents`
//! subset of `SceneQueries`; [`MonsterBody`] mirrors the monster movement
//! state consumed by `Q1MonsterMovement` in `src/movement/q1/monsters.ts`;
//! [`RandomSource`] mirrors `src/contracts/numeric.ts`. Traces reuse
//! `super::spatial_host`; link bounds reuse `super::world_host`.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use super::actor_state::{FLAG_FLY, FLAG_ONGROUND, FLAG_SWIM, SOLID_TRIGGER};
use super::entity_host::slot_for_reference;
use super::spatial_host::{Q1MoveKind, Scene, TraceHit, TraceParams, TraceShape};
use super::world_host::qc_link_bounds;
use crate::error::GuestError;
use crate::fields::{FieldTable, FieldValue};

/// Step height used by walk movement.
pub const STEP_HEIGHT: f32 = 18.0;
/// Drop distance used by bottom checks.
pub const BOTTOM_DROP: f32 = 34.0;

/// Monster movement state projected from fields and bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterBody {
    /// World-space origin.
    pub origin: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// Local collision bounds.
    pub bounds: Bounds,
    /// Linked absolute bounds.
    pub absolute_bounds: Bounds,
    /// Raw `flags` word.
    pub flags: i32,
    /// Ground contact.
    pub ground: TraceHit,
    /// Desired yaw.
    pub ideal_yaw: f32,
    /// Yaw turn speed in degrees per second.
    pub yaw_speed: f32,
    /// Current enemy, if any.
    pub enemy: Option<ActorId>,
}

/// Target snapshot for goal movement.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterTarget {
    /// Target origin.
    pub origin: Vec3,
    /// Target absolute bounds.
    pub absolute_bounds: Bounds,
}

/// Body reads behind monster movement.
pub trait MovementBodies {
    /// Read origin/angles/velocity/bounds for an actor.
    fn read(&self, actor: &ActorId) -> Option<MonsterBodySnapshot>;
    /// Read linked absolute bounds, if linked.
    fn linked(&self, actor: &ActorId) -> Option<Bounds>;
}

/// Minimal body snapshot (the ground word stays in QC fields).
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterBodySnapshot {
    /// World-space origin.
    pub origin: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// Local collision bounds.
    pub bounds: Bounds,
}

/// World services behind monster movement.
pub trait MovementWorld {
    /// Actor bound to a slot.
    fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError>;
    /// Encode an actor as an entity reference.
    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError>;
    /// Link one slot into the collision world.
    fn link(&mut self, slot: usize);
    /// Current entity row count.
    fn entity_count(&self) -> usize;
}

/// Integer random source behind goal unblocking.
pub trait RandomSource {
    /// Draw an integer in `0..bound`.
    fn next_integer(&mut self, bound: i32) -> i32;
}

/// Monster movement builtins (`walkmove`, `movetogoal`, `changeyaw`,
/// `checkbottom`).
pub struct MovementBindings<W, S, B, R> {
    world: W,
    scene: S,
    bodies: B,
    random: R,
    touch_triggers: Vec<ActorId>,
}

impl<W: MovementWorld, S: Scene, B: MovementBodies, R: RandomSource> MovementBindings<W, S, B, R> {
    /// Bind movement to its services.
    pub fn new(world: W, scene: S, bodies: B, random: R) -> Self {
        Self {
            world,
            scene,
            bodies,
            random,
            touch_triggers: Vec::new(),
        }
    }

    /// Actors whose triggers fired during linked moves (test inspection).
    #[must_use]
    pub fn touched(&self) -> &[ActorId] {
        &self.touch_triggers
    }

    /// Borrow the world.
    #[must_use]
    pub fn world(&self) -> &W {
        &self.world
    }

    /// Read the monster state for one actor.
    pub fn read_monster(
        &self,
        fields: &FieldTable,
        actor: &ActorId,
        resolve: &dyn Fn(i32) -> Option<ActorId>,
    ) -> Result<Option<MonsterBody>, GuestError> {
        let Some(body) = self.bodies.read(actor) else {
            return Ok(None);
        };
        let flags = fields.get(actor, "flags")?.as_float("flags")? as i32;
        let ground_word = fields.get(actor, "groundentity")?.as_float("groundentity")? as i32;
        let ground_actor = if ground_word == 0 { None } else { resolve(ground_word) };
        let ground = if flags & FLAG_ONGROUND == 0 {
            TraceHit::None
        } else if ground_word == 0 {
            TraceHit::World { model: 0 }
        } else if let Some(actor) = ground_actor {
            TraceHit::Actor { actor }
        } else {
            TraceHit::None
        };
        let enemy_word = fields.get(actor, "enemy")?.as_float("enemy")? as i32;
        Ok(Some(MonsterBody {
            origin: body.origin,
            angles: body.angles,
            velocity: body.velocity,
            bounds: body.bounds,
            absolute_bounds: self
                .bodies
                .linked(actor)
                .unwrap_or_else(|| qc_link_bounds(body.origin, body.bounds, flags)),
            flags,
            ground,
            ideal_yaw: fields.get(actor, "ideal_yaw")?.as_float("ideal_yaw")?,
            yaw_speed: fields.get(actor, "yaw_speed")?.as_float("yaw_speed")?,
            enemy: if enemy_word == 0 { None } else { resolve(enemy_word) },
        }))
    }

    /// `walkmove`: step `dist` units along `yaw`; returns 1 on success.
    pub fn walk_move(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        yaw: f32,
        dist: f32,
    ) -> Result<f32, GuestError> {
        let resolve = self.reference_resolver();
        let Some(mut state) = self.read_monster(fields, actor, &resolve)? else {
            return Ok(0.0);
        };
        state.ideal_yaw = yaw;
        self.change_yaw_state(&mut state);
        let radians = state.angles.y.to_radians();
        let target = Vec3 {
            x: state.origin.x + radians.cos() * dist,
            y: state.origin.y + radians.sin() * dist,
            z: state.origin.z,
        };
        let moved = self.move_step(fields, actor, &mut state, target)?;
        self.write_monster(fields, actor, &state, true)?;
        Ok(f32::from(moved))
    }

    /// `movetogoal` facing pass: turn toward `goal` without stepping.
    pub fn move_to_goal(&mut self, fields: &mut FieldTable, actor: &ActorId, goal: &ActorId) -> Result<(), GuestError> {
        let resolve = self.reference_resolver();
        let Some(mut state) = self.read_monster(fields, actor, &resolve)? else {
            return Ok(());
        };
        let Some(target) = self.read_target(fields, goal)? else {
            return Ok(());
        };
        let dx = target.origin.x - state.origin.x;
        let dy = target.origin.y - state.origin.y;
        state.ideal_yaw = dy.atan2(dx).to_degrees();
        self.change_yaw_state(&mut state);
        self.write_monster(fields, actor, &state, false)?;
        Ok(())
    }

    /// `movetogoal` with an explicit step distance.
    pub fn move_to_goal_dist(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        goal: &ActorId,
        dist: f32,
    ) -> Result<(), GuestError> {
        let resolve = self.reference_resolver();
        let Some(mut state) = self.read_monster(fields, actor, &resolve)? else {
            return Ok(());
        };
        if state.flags & (FLAG_ONGROUND | FLAG_FLY | FLAG_SWIM) == 0 {
            return Ok(());
        }
        let Some(target) = self.read_target(fields, goal)? else {
            return Ok(());
        };
        let dx = target.origin.x - state.origin.x;
        let dy = target.origin.y - state.origin.y;
        state.ideal_yaw = dy.atan2(dx).to_degrees();
        self.change_yaw_state(&mut state);
        let radians = state.angles.y.to_radians();
        let step = Vec3 {
            x: state.origin.x + radians.cos() * dist,
            y: state.origin.y + radians.sin() * dist,
            z: state.origin.z,
        };
        if !self.move_step(fields, actor, &mut state, step)? {
            // Blocked: the donor turns around with a random bias instead of
            // pushing into the wall.
            state.ideal_yaw = state.angles.y + 180.0 + f32::from(self.random.next_integer(90) - 45);
            self.change_yaw_state(&mut state);
        }
        self.write_monster(fields, actor, &state, true)?;
        Ok(())
    }

    /// `changeyaw`: turn the current yaw toward the ideal yaw.
    pub fn change_yaw(&mut self, fields: &mut FieldTable, actor: &ActorId) -> Result<(), GuestError> {
        let resolve = self.reference_resolver();
        let Some(mut state) = self.read_monster(fields, actor, &resolve)? else {
            return Ok(());
        };
        self.change_yaw_state(&mut state);
        self.write_monster(fields, actor, &state, false)?;
        Ok(())
    }

    /// `checkbottom`: whether the actor stands over solid ground.
    pub fn check_bottom(&self, fields: &FieldTable, actor: &ActorId) -> Result<bool, GuestError> {
        let resolve = self.reference_resolver();
        let Some(state) = self.read_monster(fields, actor, &resolve)? else {
            return Ok(false);
        };
        if state.flags & (FLAG_FLY | FLAG_SWIM) != 0 {
            return Ok(true);
        }
        let corners = [
            (state.bounds.min.x, state.bounds.min.y),
            (state.bounds.min.x, state.bounds.max.y),
            (state.bounds.max.x, state.bounds.min.y),
            (state.bounds.max.x, state.bounds.max.y),
            (0.0, 0.0),
        ];
        for (dx, dy) in corners {
            let start = Vec3 {
                x: state.origin.x + dx,
                y: state.origin.y + dy,
                z: state.origin.z,
            };
            let trace = self.scene.trace(&TraceParams {
                start,
                end: Vec3 {
                    x: start.x,
                    y: start.y,
                    z: start.z - BOTTOM_DROP,
                },
                shape: TraceShape::Point,
                move_kind: Q1MoveKind::Normal,
                pass_actor: Some(actor.clone()),
            })?;
            if trace.fraction == 1.0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn read_target(&self, fields: &FieldTable, goal: &ActorId) -> Result<Option<MonsterTarget>, GuestError> {
        let Some(body) = self.bodies.read(goal) else {
            return Ok(None);
        };
        let absolute_bounds = match (fields.get(goal, "absmin"), fields.get(goal, "absmax")) {
            (Ok(_), Ok(_)) => Bounds {
                min: fields.get(goal, "absmin")?.as_vector("absmin")?,
                max: fields.get(goal, "absmax")?.as_vector("absmax")?,
            },
            _ => self
                .bodies
                .linked(goal)
                .unwrap_or_else(|| qc_link_bounds(body.origin, body.bounds, 0)),
        };
        Ok(Some(MonsterTarget {
            origin: body.origin,
            absolute_bounds,
        }))
    }

    fn move_step(
        &mut self,
        fields: &FieldTable,
        actor: &ActorId,
        state: &mut MonsterBody,
        target: Vec3,
    ) -> Result<bool, GuestError> {
        let direct = self.scene.trace(&TraceParams {
            start: state.origin,
            end: target,
            shape: TraceShape::Box { bounds: state.bounds },
            move_kind: Q1MoveKind::Normal,
            pass_actor: Some(actor.clone()),
        })?;
        if direct.fraction == 1.0 {
            state.origin = target;
            return Ok(true);
        }
        // `SV_movestep` stair move: up, across, then down.
        let up = Vec3 {
            x: state.origin.x,
            y: state.origin.y,
            z: state.origin.z + STEP_HEIGHT,
        };
        let up_trace = self.scene.trace(&TraceParams {
            start: state.origin,
            end: up,
            shape: TraceShape::Box { bounds: state.bounds },
            move_kind: Q1MoveKind::Normal,
            pass_actor: Some(actor.clone()),
        })?;
        if up_trace.all_solid || up_trace.start_solid {
            return Ok(false);
        }
        let across = Vec3 {
            x: target.x,
            y: target.y,
            z: up_trace.end.z,
        };
        let across_trace = self.scene.trace(&TraceParams {
            start: up_trace.end,
            end: across,
            shape: TraceShape::Box { bounds: state.bounds },
            move_kind: Q1MoveKind::Normal,
            pass_actor: Some(actor.clone()),
        })?;
        if across_trace.fraction == 1.0 {
            let down = Vec3 {
                x: across_trace.end.x,
                y: across_trace.end.y,
                z: across_trace.end.z - STEP_HEIGHT,
            };
            let down_trace = self.scene.trace(&TraceParams {
                start: across_trace.end,
                end: down,
                shape: TraceShape::Box { bounds: state.bounds },
                move_kind: Q1MoveKind::Normal,
                pass_actor: Some(actor.clone()),
            })?;
            state.origin = down_trace.end;
            state.ground = down_trace.hit;
            let _ = fields;
            return Ok(true);
        }
        Ok(false)
    }

    fn change_yaw_state(&self, state: &mut MonsterBody) {
        let mut delta = state.ideal_yaw - state.angles.y;
        while delta > 180.0 {
            delta -= 360.0;
        }
        while delta < -180.0 {
            delta += 360.0;
        }
        // Donor `ChangeYaw` runs per 0.1s think; clamp the turn to one step.
        let step = state.yaw_speed * 0.1;
        let turn = delta.clamp(-step, step);
        state.angles.y += turn;
    }

    fn write_monster(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        state: &MonsterBody,
        link: bool,
    ) -> Result<(), GuestError> {
        fields.set(actor, "origin", FieldValue::Vector(state.origin))?;
        fields.set(actor, "angles", FieldValue::Vector(state.angles))?;
        // `SV_movestep` retains the raw ground word until a landing supplies
        // a contact.
        match &state.ground {
            TraceHit::Actor { actor: ground } => {
                let reference = self.world.reference(ground)?;
                fields.set(actor, "groundentity", FieldValue::Float(reference as f32))?;
            }
            TraceHit::World { .. } => {
                fields.set(actor, "groundentity", FieldValue::Float(0.0))?;
            }
            TraceHit::None => {}
        }
        fields.set(actor, "flags", FieldValue::Float(state.flags as f32))?;
        fields.set(actor, "ideal_yaw", FieldValue::Float(state.ideal_yaw))?;
        fields.set(actor, "yaw_speed", FieldValue::Float(state.yaw_speed))?;
        if link {
            let reference = self.world.reference(actor)?;
            let slot = slot_for_reference(reference, self.world.entity_count())?;
            self.world.link(slot);
            self.touch_triggers.push(actor.clone());
        }
        Ok(())
    }

    fn reference_resolver(&self) -> impl Fn(i32) -> Option<ActorId> + '_ {
        |_reference| None
    }
}

/// `self`/`other`/`time` words set around a touch dispatch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TouchGlobals {
    /// `self` entity reference.
    pub self_reference: i32,
    /// `other` entity reference.
    pub other_reference: i32,
    /// `time` at dispatch.
    pub time_seconds: f32,
}

/// World services behind touch dispatch.
pub trait TouchWorld {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Source slot owned by an actor.
    fn source_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Encode an actor as an entity reference.
    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError>;
}

/// Touch contact between two actors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchContact {
    /// Touching entity.
    pub this: ActorId,
    /// Touched entity.
    pub other: ActorId,
}

/// Dispatch a touch contact: triggers (`solid == 1`) with a `touch`
/// callback run with `self`/`other`/`time` set. Returns whether the
/// callback ran. `invoke` executes the callback word.
pub fn dispatch_touch<W: TouchWorld>(
    world: &W,
    fields: &FieldTable,
    globals: &mut TouchGlobals,
    contact: &TouchContact,
    server_time_seconds: f32,
    invoke: &mut dyn FnMut(i32) -> Result<(), GuestError>,
) -> Result<bool, GuestError> {
    if !world.is_live(&contact.this) || !world.is_live(&contact.other) {
        return Ok(false);
    }
    if world.source_slot(&contact.this).is_none() {
        return Ok(false);
    }
    if fields.get(&contact.this, "solid")?.as_float("solid")? != SOLID_TRIGGER {
        return Ok(false);
    }
    let callback = fields.get(&contact.this, "touch")?.as_float("touch")? as i32;
    if callback == 0 {
        return Ok(false);
    }
    let saved = globals.clone();
    globals.self_reference = world.reference(&contact.this)?;
    globals.other_reference = world.reference(&contact.other)?;
    globals.time_seconds = server_time_seconds;
    let result = invoke(callback);
    *globals = saved;
    result?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldLayout;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Plane};
    use std::collections::HashMap;

    struct FakeWorld {
        actors: Vec<ActorId>,
        links: Vec<usize>,
    }

    impl MovementWorld for FakeWorld {
        fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError> {
            self.actors
                .get(slot)
                .cloned()
                .ok_or_else(|| GuestError::invalid("free slot"))
        }

        fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
            self.actors
                .iter()
                .position(|entry| entry == actor)
                .map(|slot| slot as i32)
                .ok_or_else(|| GuestError::invalid("stale actor"))
        }

        fn link(&mut self, slot: usize) {
            self.links.push(slot);
        }

        fn entity_count(&self) -> usize {
            self.actors.len()
        }
    }

    impl TouchWorld for FakeWorld {
        fn is_live(&self, actor: &ActorId) -> bool {
            self.actors.contains(actor)
        }

        fn source_slot(&self, actor: &ActorId) -> Option<usize> {
            self.actors.iter().position(|entry| entry == actor)
        }

        fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
            MovementWorld::reference(self, actor)
        }
    }

    struct FakeScene {
        blocked: bool,
    }

    impl Scene for FakeScene {
        fn trace(&self, params: &TraceParams) -> Result<super::super::spatial_host::TraceResult, GuestError> {
            let going_down = params.end.z < params.start.z;
            let fraction = if self.blocked && !going_down { 0.0 } else { 1.0 };
            Ok(super::super::spatial_host::TraceResult {
                fraction,
                all_solid: false,
                start_solid: false,
                in_water: false,
                in_open: true,
                end: if fraction == 1.0 { params.end } else { params.start },
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: params.end.z,
                },
                hit: if going_down {
                    TraceHit::World { model: 0 }
                } else {
                    TraceHit::None
                },
            })
        }

        fn point_contents(&self, _point: Vec3) -> Result<f32, GuestError> {
            Ok(0.0)
        }
    }

    struct FakeBodies {
        bodies: HashMap<ActorId, MonsterBodySnapshot>,
    }

    impl MovementBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<MonsterBodySnapshot> {
            self.bodies.get(actor).cloned()
        }

        fn linked(&self, _actor: &ActorId) -> Option<Bounds> {
            None
        }
    }

    struct FakeRandom {
        next: i32,
    }

    impl RandomSource for FakeRandom {
        fn next_integer(&mut self, bound: i32) -> i32 {
            self.next = (self.next + 1) % bound.max(1);
            self.next
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity()
            .field("flags", "float")
            .field("groundentity", "float")
            .field("enemy", "float")
            .field("ideal_yaw", "float")
            .field("yaw_speed", "float")
            .field("touch", "float")
            .field("absmin", "vector")
            .field("absmax", "vector")
    }

    fn harness(blocked: bool) -> (IdentityOwner, FieldTable, FakeWorld, FakeScene, FakeBodies) {
        let owner = IdentityOwner::create("movement-host").unwrap();
        let monster = owner.actor(1, 1);
        let goal = owner.actor(2, 1);
        let mut fields = FieldTable::new();
        for actor in [&monster, &goal] {
            fields.allocate(actor, &layout()).unwrap();
        }
        fields
            .set(&monster, "origin", FieldValue::Vector(vec3(0.0, 0.0, 0.0)))
            .unwrap();
        fields.set(&monster, "flags", FieldValue::Float(512.0)).unwrap();
        fields.set(&monster, "ideal_yaw", FieldValue::Float(0.0)).unwrap();
        fields.set(&monster, "yaw_speed", FieldValue::Float(200.0)).unwrap();
        fields
            .set(&goal, "origin", FieldValue::Vector(vec3(64.0, 0.0, 0.0)))
            .unwrap();
        let mut bodies = HashMap::new();
        bodies.insert(
            monster.clone(),
            MonsterBodySnapshot {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
            },
        );
        bodies.insert(
            goal.clone(),
            MonsterBodySnapshot {
                origin: vec3(64.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: Bounds {
                    min: vec3(-1.0, -1.0, -1.0),
                    max: vec3(1.0, 1.0, 1.0),
                },
            },
        );
        let world = FakeWorld {
            actors: vec![owner.actor(0, 1), monster, goal],
            links: Vec::new(),
        };
        (owner, fields, world, FakeScene { blocked }, FakeBodies { bodies })
    }

    #[test]
    fn walk_move_steps_and_links() {
        let (owner, mut fields, world, scene, bodies) = harness(false);
        let mut bindings = MovementBindings::new(world, scene, bodies, FakeRandom { next: 0 });
        let monster = owner.actor(1, 1);
        let moved = bindings.walk_move(&mut fields, &monster, 0.0, 8.0).unwrap();
        assert_eq!(moved, 1.0);
        assert_eq!(
            fields.get(&monster, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(8.0, 0.0, 0.0)
        );
        assert_eq!(bindings.touched(), &[monster]);
        assert_eq!(bindings.world().links, vec![1]);
    }

    #[test]
    fn walk_move_reports_blocks() {
        let (owner, mut fields, world, scene, bodies) = harness(true);
        let mut bindings = MovementBindings::new(world, scene, bodies, FakeRandom { next: 0 });
        let monster = owner.actor(1, 1);
        let moved = bindings.walk_move(&mut fields, &monster, 0.0, 8.0).unwrap();
        assert_eq!(moved, 0.0);
        assert_eq!(
            fields.get(&monster, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn change_yaw_turns_toward_ideal() {
        let (owner, mut fields, world, scene, bodies) = harness(false);
        let mut bindings = MovementBindings::new(world, scene, bodies, FakeRandom { next: 0 });
        let monster = owner.actor(1, 1);
        fields.set(&monster, "ideal_yaw", FieldValue::Float(90.0)).unwrap();
        bindings.change_yaw(&mut fields, &monster).unwrap();
        let angles = fields.get(&monster, "angles").unwrap().as_vector("angles").unwrap();
        assert_eq!(angles.y, 20.0);
    }

    #[test]
    fn move_to_goal_faces_and_steps() {
        let (owner, mut fields, world, scene, bodies) = harness(false);
        let mut bindings = MovementBindings::new(world, scene, bodies, FakeRandom { next: 0 });
        let (monster, goal) = (owner.actor(1, 1), owner.actor(2, 1));
        bindings.move_to_goal(&mut fields, &monster, &goal).unwrap();
        bindings.move_to_goal_dist(&mut fields, &monster, &goal, 8.0).unwrap();
        let origin = fields.get(&monster, "origin").unwrap().as_vector("origin").unwrap();
        assert_eq!(origin, vec3(8.0, 0.0, 0.0));
    }

    #[test]
    fn check_bottom_samples_corners() {
        let (owner, fields, world, scene, bodies) = harness(false);
        let bindings = MovementBindings::new(world, scene, bodies, FakeRandom { next: 0 });
        assert!(bindings.check_bottom(&fields, &owner.actor(1, 1)).unwrap());
    }

    #[test]
    fn touch_dispatch_runs_triggers_with_saved_globals() {
        let (owner, mut fields, world, _scene, _bodies) = harness(false);
        let (trigger, other) = (owner.actor(1, 1), owner.actor(2, 1));
        fields.set(&trigger, "solid", FieldValue::Float(1.0)).unwrap();
        fields.set(&trigger, "touch", FieldValue::Float(42.0)).unwrap();
        let mut globals = TouchGlobals::default();
        let mut ran = Vec::new();
        let dispatched = dispatch_touch(
            &world,
            &fields,
            &mut globals,
            &TouchContact {
                this: trigger.clone(),
                other: other.clone(),
            },
            3.5,
            &mut |callback| {
                ran.push(callback);
                Ok(())
            },
        )
        .unwrap();
        assert!(dispatched);
        assert_eq!(ran, vec![42]);
        assert_eq!(globals, TouchGlobals::default());
        fields.set(&trigger, "touch", FieldValue::Float(0.0)).unwrap();
        let dispatched = dispatch_touch(
            &world,
            &fields,
            &mut globals,
            &TouchContact { this: trigger, other },
            3.5,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert!(!dispatched);
    }
}
