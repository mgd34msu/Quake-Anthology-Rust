//! `SV_Physics_Pusher` field projection with raw QC dispatch.
//!
//! Ported from donor `src/compat/qc/pusher-host.ts`
//! (`createQcPusherServices`).
//!
//! Local mirrors: [`Q1PhysicsEntity`] and [`PusherState`] mirror
//! `Q1PhysicsEntity` from `src/movement/q1/types.ts`; [`PusherWorld`],
//! [`PusherBodies`], [`ForeignPusher`], and [`PusherPhysics`] mirror the
//! world/body/foreign/physical service split consumed by the donor.
//! Ground contacts reuse `super::spatial_host`; flag constants reuse
//! `super::actor_state`.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use super::actor_state::{FLAG_ONGROUND, SOLID_BSP, SOLID_NOT, SOLID_TRIGGER};
use super::presentation_host::ApiKind;
use super::spatial_host::TraceHit;
use crate::error::GuestError;
use crate::fields::{FieldTable, FieldValue};

/// Pusher solidity projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PusherSolid {
    /// Non-solid.
    Not,
    /// Trigger volume.
    Trigger,
    /// Brush model.
    Bsp,
    /// Sliding box.
    SlideBox,
    /// Bounding-box solid.
    Box,
}

/// Pusher callback selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PusherCallback {
    /// `think` callback.
    Think,
    /// `blocked` callback.
    Blocked,
}

/// Full pusher physics state projected from QC words.
#[derive(Debug, Clone, PartialEq)]
pub struct PusherState {
    /// World-space origin.
    pub origin: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// View angles.
    pub view_angles: Vec3,
    /// Punch angles (zero on QuakeWorld).
    pub punch_angles: Vec3,
    /// Raw `movetype` word.
    pub move_type: f32,
    /// Raw `flags` word.
    pub flags: i32,
    /// Ground contact.
    pub ground: TraceHit,
    /// Water depth level.
    pub water_level: f32,
    /// Water contents type.
    pub water_type: f32,
    /// Teleport time.
    pub teleport_time_seconds: f32,
    /// Water-jump direction.
    pub water_jump_direction: Vec3,
    /// Ideal pitch (zero on QuakeWorld).
    pub ideal_pitch: f32,
    /// Fix-angle flag.
    pub fix_angle: bool,
    /// Health.
    pub health: f32,
}

/// Physics entity consumed by the shared pusher driver.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PhysicsEntity {
    /// Entity actor.
    pub actor: ActorId,
    /// Local collision bounds.
    pub bounds: Bounds,
    /// Linked absolute bounds.
    pub absolute_bounds: Bounds,
    /// Projected solidity.
    pub solid: PusherSolid,
    /// Local time in seconds.
    pub local_time_seconds: f32,
    /// Next think time in seconds.
    pub next_think_seconds: f32,
    /// Pusher state.
    pub state: PusherState,
}

/// World services behind pusher projection.
pub trait PusherWorld {
    /// Source slot owned by an actor, if the actor owns a QC row.
    fn source_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Encode an actor as an entity reference.
    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError>;
    /// Link one source slot into the collision world.
    fn link_slot(&mut self, slot: usize);
}

/// Body reads behind pusher projection.
pub trait PusherBodies {
    /// Minimal body snapshot: origin, velocity, angles, bounds.
    fn read(&self, actor: &ActorId) -> Option<PusherBodySnapshot>;
    /// Ground actor from the shared body table.
    fn ground(&self, actor: &ActorId) -> Option<ActorId>;
    /// Link a foreign (non-QC) actor.
    fn link_foreign(&mut self, actor: &ActorId);
}

/// Minimal body snapshot behind pusher reads.
#[derive(Debug, Clone, PartialEq)]
pub struct PusherBodySnapshot {
    /// World-space origin.
    pub origin: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Local collision bounds.
    pub bounds: Bounds,
}

/// Foreign-actor projection owned by the application.
pub trait ForeignPusher {
    /// Read a foreign actor's physics entity.
    fn read(&self, actor: &ActorId) -> Option<Q1PhysicsEntity>;
    /// Write a foreign actor's physics entity.
    fn write(&mut self, entity: &Q1PhysicsEntity);
}

/// Shared physical pusher driver (the donor `physical` services).
pub trait PusherPhysics {
    /// Link notification after a move.
    fn on_link(&mut self, actor: &ActorId, touch: bool);
    /// Blocked notification (unused by the QC projection path).
    fn on_blocked(&mut self, actor: &ActorId, other: Option<&ActorId>);
    /// Enable or disable collision for an actor.
    fn set_collision_enabled(&mut self, actor: &ActorId, enabled: bool);
}

/// Callback invocation (the donor `invoke` option or direct VM dispatch).
pub trait PusherInvoker {
    /// Invoke a pusher callback for an actor.
    fn invoke(
        &mut self,
        actor: &ActorId,
        callback: PusherCallback,
        callback_word: i32,
        other: Option<&ActorId>,
        server_time_seconds: f32,
    ) -> Result<(), GuestError>;
}

/// QC pusher services: raw projection plus callback dispatch.
pub struct QcPusherServices<W, B, F, P, I> {
    world: W,
    bodies: B,
    foreign: F,
    physics: P,
    invoker: I,
    api: ApiKind,
    server_time_seconds: f32,
    touch_triggers: Vec<ActorId>,
}

impl<W: PusherWorld, B: PusherBodies, F: ForeignPusher, P: PusherPhysics, I: PusherInvoker>
    QcPusherServices<W, B, F, P, I>
{
    /// Build pusher services over their dependencies.
    pub fn new(
        world: W,
        bodies: B,
        foreign: F,
        physics: P,
        invoker: I,
        api: ApiKind,
        server_time_seconds: f32,
    ) -> Self {
        Self {
            world,
            bodies,
            foreign,
            physics,
            invoker,
            api,
            server_time_seconds,
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

    /// Read a physics entity, delegating foreign actors to the application.
    pub fn read(&self, fields: &FieldTable, actor: &ActorId) -> Result<Option<Q1PhysicsEntity>, GuestError> {
        if self.world.source_slot(actor).is_none() {
            return Ok(self.foreign.read(actor));
        }
        if !self.world.is_live(actor) {
            return Ok(None);
        }
        let Some(body) = self.bodies.read(actor) else {
            return Ok(None);
        };
        let flags = fields.get(actor, "flags")?.as_float("flags")? as i32;
        let ground_word = fields.get(actor, "groundentity")?.as_float("groundentity")? as i32;
        let ground_body = if ground_word == 0 {
            None
        } else {
            self.bodies.ground(actor)
        };
        let ground = if flags & FLAG_ONGROUND == 0 {
            TraceHit::None
        } else if ground_word == 0 {
            TraceHit::World { model: 0 }
        } else if let Some(actor) = ground_body {
            TraceHit::Actor { actor }
        } else {
            TraceHit::None
        };
        let solid_word = fields.get(actor, "solid")?.as_float("solid")?;
        let solid = if solid_word == SOLID_NOT {
            PusherSolid::Not
        } else if solid_word == SOLID_TRIGGER {
            PusherSolid::Trigger
        } else if solid_word == SOLID_BSP {
            PusherSolid::Bsp
        } else if solid_word == 3.0 {
            PusherSolid::SlideBox
        } else {
            PusherSolid::Box
        };
        let quakeworld = self.api == ApiKind::QuakeWorld;
        Ok(Some(Q1PhysicsEntity {
            actor: actor.clone(),
            bounds: body.bounds,
            absolute_bounds: Bounds {
                min: fields.get(actor, "absmin")?.as_vector("absmin")?,
                max: fields.get(actor, "absmax")?.as_vector("absmax")?,
            },
            solid,
            local_time_seconds: fields.get(actor, "ltime")?.as_float("ltime")?,
            next_think_seconds: fields.get(actor, "nextthink")?.as_float("nextthink")?,
            state: PusherState {
                origin: body.origin,
                velocity: body.velocity,
                angles: body.angles,
                old_origin: fields.get(actor, "oldorigin")?.as_vector("oldorigin")?,
                angular_velocity: fields.get(actor, "avelocity")?.as_vector("avelocity")?,
                view_angles: fields.get(actor, "v_angle")?.as_vector("v_angle")?,
                punch_angles: if quakeworld {
                    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
                } else {
                    fields.get(actor, "punchangle")?.as_vector("punchangle")?
                },
                move_type: fields.get(actor, "movetype")?.as_float("movetype")?,
                flags,
                ground,
                water_level: fields.get(actor, "waterlevel")?.as_float("waterlevel")?,
                water_type: fields.get(actor, "watertype")?.as_float("watertype")?,
                teleport_time_seconds: fields.get(actor, "teleport_time")?.as_float("teleport_time")?,
                water_jump_direction: fields.get(actor, "movedir")?.as_vector("movedir")?,
                ideal_pitch: if quakeworld {
                    0.0
                } else {
                    fields.get(actor, "idealpitch")?.as_float("idealpitch")?
                },
                fix_angle: fields.get(actor, "fixangle")?.as_float("fixangle")? != 0.0,
                health: fields.get(actor, "health")?.as_float("health")?,
            },
        }))
    }

    /// Write a physics entity, delegating foreign actors to the application.
    pub fn write(&mut self, fields: &mut FieldTable, entity: &Q1PhysicsEntity) -> Result<(), GuestError> {
        if self.world.source_slot(&entity.actor).is_none() {
            self.foreign.write(entity);
            return Ok(());
        }
        fields.set(&entity.actor, "origin", FieldValue::Vector(entity.state.origin))?;
        fields.set(&entity.actor, "angles", FieldValue::Vector(entity.state.angles))?;
        fields.set(&entity.actor, "flags", FieldValue::Float(entity.state.flags as f32))?;
        fields.set(&entity.actor, "mins", FieldValue::Vector(entity.bounds.min))?;
        fields.set(&entity.actor, "maxs", FieldValue::Vector(entity.bounds.max))?;
        fields.set(&entity.actor, "ltime", FieldValue::Float(entity.local_time_seconds))?;
        fields.set(&entity.actor, "nextthink", FieldValue::Float(entity.next_think_seconds))?;
        // `SV_PushMove` clears onground but does not rewrite the raw
        // groundentity word.
        Ok(())
    }

    /// Link an actor after a move; source rows link by slot, foreign rows
    /// link through the shared body table.
    pub fn link(&mut self, actor: &ActorId, touch: bool) {
        match self.world.source_slot(actor) {
            Some(slot) => self.world.link_slot(slot),
            None => self.bodies.link_foreign(actor),
        }
        self.physics.on_link(actor, touch);
        if touch {
            self.touch_triggers.push(actor.clone());
        }
    }

    /// Dispatch the `blocked` callback; an unset callback word is a no-op.
    pub fn blocked(&mut self, fields: &FieldTable, actor: &ActorId, other: Option<&ActorId>) -> Result<(), GuestError> {
        self.invoke(fields, actor, PusherCallback::Blocked, other)
    }

    /// Enable or disable collision. Source rows flip the `solid` word
    /// between brush and non-solid before delegating to the driver.
    pub fn collision_enabled(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        enabled: bool,
    ) -> Result<(), GuestError> {
        if self.world.source_slot(actor).is_some() {
            fields.set(
                actor,
                "solid",
                FieldValue::Float(if enabled { SOLID_BSP } else { SOLID_NOT }),
            )?;
        }
        self.physics.set_collision_enabled(actor, enabled);
        Ok(())
    }

    /// Dispatch the `think` callback.
    pub fn think(&mut self, fields: &FieldTable, actor: &ActorId) -> Result<(), GuestError> {
        self.invoke(fields, actor, PusherCallback::Think, None)
    }

    fn invoke(
        &mut self,
        fields: &FieldTable,
        actor: &ActorId,
        callback: PusherCallback,
        other: Option<&ActorId>,
    ) -> Result<(), GuestError> {
        if self.world.source_slot(actor).is_none() {
            return Err(GuestError::invalid("pusher callback requires a live QC actor"));
        }
        let name = match callback {
            PusherCallback::Think => "think",
            PusherCallback::Blocked => "blocked",
        };
        let word = fields.get(actor, name)?.as_float(name)? as i32;
        if word == 0 && callback == PusherCallback::Blocked {
            return Ok(());
        }
        self.invoker
            .invoke(actor, callback, word, other, self.server_time_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldLayout;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use std::collections::HashMap;

    struct FakeWorld {
        actors: Vec<ActorId>,
        links: Vec<usize>,
    }

    impl PusherWorld for FakeWorld {
        fn source_slot(&self, actor: &ActorId) -> Option<usize> {
            self.actors.iter().position(|entry| entry == actor)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.actors.contains(actor)
        }

        fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
            self.source_slot(actor)
                .map(|slot| slot as i32)
                .ok_or_else(|| GuestError::invalid("stale actor"))
        }

        fn link_slot(&mut self, slot: usize) {
            self.links.push(slot);
        }
    }

    struct FakeBodies {
        bodies: HashMap<ActorId, PusherBodySnapshot>,
        foreign_links: Vec<ActorId>,
    }

    impl PusherBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<PusherBodySnapshot> {
            self.bodies.get(actor).cloned()
        }

        fn ground(&self, _actor: &ActorId) -> Option<ActorId> {
            None
        }

        fn link_foreign(&mut self, actor: &ActorId) {
            self.foreign_links.push(actor.clone());
        }
    }

    struct FakeForeign {
        entities: HashMap<ActorId, Q1PhysicsEntity>,
    }

    impl ForeignPusher for FakeForeign {
        fn read(&self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
            self.entities.get(actor).cloned()
        }

        fn write(&mut self, entity: &Q1PhysicsEntity) {
            self.entities.insert(entity.actor.clone(), entity.clone());
        }
    }

    struct FakePhysics {
        links: Vec<(ActorId, bool)>,
        blocked: Vec<(ActorId, Option<ActorId>)>,
        collision: Vec<(ActorId, bool)>,
    }

    impl PusherPhysics for FakePhysics {
        fn on_link(&mut self, actor: &ActorId, touch: bool) {
            self.links.push((actor.clone(), touch));
        }

        fn on_blocked(&mut self, actor: &ActorId, other: Option<&ActorId>) {
            self.blocked.push((actor.clone(), other.cloned()));
        }

        fn set_collision_enabled(&mut self, actor: &ActorId, enabled: bool) {
            self.collision.push((actor.clone(), enabled));
        }
    }

    struct FakeInvoker {
        calls: Vec<(ActorId, PusherCallback, i32, Option<ActorId>, f32)>,
    }

    impl PusherInvoker for FakeInvoker {
        fn invoke(
            &mut self,
            actor: &ActorId,
            callback: PusherCallback,
            callback_word: i32,
            other: Option<&ActorId>,
            server_time_seconds: f32,
        ) -> Result<(), GuestError> {
            self.calls.push((
                actor.clone(),
                callback,
                callback_word,
                other.cloned(),
                server_time_seconds,
            ));
            Ok(())
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity()
            .field("flags", "float")
            .field("groundentity", "float")
            .field("absmin", "vector")
            .field("absmax", "vector")
            .field("mins", "vector")
            .field("maxs", "vector")
            .field("ltime", "float")
            .field("oldorigin", "vector")
            .field("avelocity", "vector")
            .field("v_angle", "vector")
            .field("punchangle", "vector")
            .field("movetype", "float")
            .field("waterlevel", "float")
            .field("watertype", "float")
            .field("teleport_time", "float")
            .field("movedir", "vector")
            .field("idealpitch", "float")
            .field("fixangle", "float")
            .field("health", "float")
            .field("think", "float")
            .field("blocked", "float")
    }

    fn harness() -> (IdentityOwner, FieldTable, FakeWorld, FakeBodies) {
        let owner = IdentityOwner::create("pusher-host").unwrap();
        let pusher = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        fields.allocate(&pusher, &layout()).unwrap();
        fields.set(&pusher, "solid", FieldValue::Float(4.0)).unwrap();
        fields
            .set(&pusher, "origin", FieldValue::Vector(vec3(0.0, 0.0, 0.0)))
            .unwrap();
        fields.set(&pusher, "flags", FieldValue::Float(512.0)).unwrap();
        fields.set(&pusher, "movetype", FieldValue::Float(7.0)).unwrap();
        fields.set(&pusher, "health", FieldValue::Float(100.0)).unwrap();
        fields.set(&pusher, "think", FieldValue::Float(11.0)).unwrap();
        let mut bodies = HashMap::new();
        bodies.insert(
            pusher.clone(),
            PusherBodySnapshot {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 50.0),
                angles: vec3(0.0, 0.0, 0.0),
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -8.0),
                    max: vec3(64.0, 64.0, 8.0),
                },
            },
        );
        let world = FakeWorld {
            actors: vec![owner.actor(0, 1), pusher],
            links: Vec::new(),
        };
        (
            owner,
            fields,
            world,
            FakeBodies {
                bodies,
                foreign_links: Vec::new(),
            },
        )
    }

    fn services(
        world: FakeWorld,
        bodies: FakeBodies,
        api: ApiKind,
    ) -> QcPusherServices<FakeWorld, FakeBodies, FakeForeign, FakePhysics, FakeInvoker> {
        QcPusherServices::new(
            world,
            bodies,
            FakeForeign {
                entities: HashMap::new(),
            },
            FakePhysics {
                links: Vec::new(),
                blocked: Vec::new(),
                collision: Vec::new(),
            },
            FakeInvoker { calls: Vec::new() },
            api,
            12.0,
        )
    }

    #[test]
    fn read_projects_full_pusher_state() {
        let (owner, fields, world, bodies) = harness();
        let services = services(world, bodies, ApiKind::NetQuake);
        let entity = services.read(&fields, &owner.actor(1, 1)).unwrap().unwrap();
        assert_eq!(entity.solid, PusherSolid::Bsp);
        assert_eq!(entity.state.move_type, 7.0);
        assert_eq!(entity.state.velocity, vec3(0.0, 0.0, 50.0));
        assert_eq!(entity.state.ground, TraceHit::World { model: 0 });
        assert!(!entity.state.fix_angle);
        assert_eq!(services.world().reference(&owner.actor(1, 1)).unwrap(), 1);
    }

    #[test]
    fn quakeworld_zeroes_punch_and_pitch() {
        let (owner, mut fields, world, bodies) = harness();
        let pusher = owner.actor(1, 1);
        fields
            .set(&pusher, "punchangle", FieldValue::Vector(vec3(1.0, 2.0, 3.0)))
            .unwrap();
        fields.set(&pusher, "idealpitch", FieldValue::Float(45.0)).unwrap();
        let nq = services(world, bodies, ApiKind::NetQuake);
        let before = nq.read(&fields, &pusher).unwrap().unwrap();
        assert_eq!(before.state.punch_angles, vec3(1.0, 2.0, 3.0));
        let (owner, fields, world, bodies) = harness();
        let qw = services(world, bodies, ApiKind::QuakeWorld);
        let after = qw.read(&fields, &owner.actor(1, 1)).unwrap().unwrap();
        assert_eq!(after.state.punch_angles, vec3(0.0, 0.0, 0.0));
        assert_eq!(after.state.ideal_pitch, 0.0);
    }

    #[test]
    fn write_stores_words_without_ground_rewrite() {
        let (owner, mut fields, world, bodies) = harness();
        let mut services = services(world, bodies, ApiKind::NetQuake);
        let pusher = owner.actor(1, 1);
        fields.set(&pusher, "groundentity", FieldValue::Float(7.0)).unwrap();
        let mut entity = services.read(&fields, &pusher).unwrap().unwrap();
        entity.state.origin = vec3(0.0, 0.0, 32.0);
        entity.next_think_seconds = 20.0;
        services.write(&mut fields, &entity).unwrap();
        assert_eq!(
            fields.get(&pusher, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(0.0, 0.0, 32.0)
        );
        assert_eq!(fields.get(&pusher, "nextthink").unwrap(), &FieldValue::Float(20.0));
        assert_eq!(fields.get(&pusher, "groundentity").unwrap(), &FieldValue::Float(7.0));
    }

    #[test]
    fn think_blocked_and_links_dispatch() {
        let (owner, mut fields, world, bodies) = harness();
        let mut services = services(world, bodies, ApiKind::NetQuake);
        let (pusher, other) = (owner.actor(1, 1), owner.actor(2, 1));
        services.think(&fields, &pusher).unwrap();
        // Unset blocked word is a no-op.
        services.blocked(&fields, &pusher, Some(&other)).unwrap();
        fields.set(&pusher, "blocked", FieldValue::Float(13.0)).unwrap();
        services.blocked(&fields, &pusher, Some(&other)).unwrap();
        services.link(&pusher, true);
        assert_eq!(services.touched(), &[pusher]);
        assert!(services.world().links.contains(&1));
    }

    #[test]
    fn collision_toggle_flips_solid_word() {
        let (owner, mut fields, world, bodies) = harness();
        let mut services = services(world, bodies, ApiKind::NetQuake);
        let pusher = owner.actor(1, 1);
        services.collision_enabled(&mut fields, &pusher, false).unwrap();
        assert_eq!(fields.get(&pusher, "solid").unwrap(), &FieldValue::Float(0.0));
        services.collision_enabled(&mut fields, &pusher, true).unwrap();
        assert_eq!(fields.get(&pusher, "solid").unwrap(), &FieldValue::Float(4.0));
        assert!(services.invoke(&fields, &pusher, PusherCallback::Think, None).is_ok());
    }

    #[test]
    fn foreign_actors_delegate() {
        let (owner, fields, world, bodies) = harness();
        let mut services = services(world, bodies, ApiKind::NetQuake);
        let alien = owner.actor(9, 1);
        assert!(services.read(&fields, &alien).unwrap().is_none());
        assert!(services.think(&fields, &alien).is_err());
        services.link(&alien, false);
        assert!(services.touched().is_empty());
    }
}
