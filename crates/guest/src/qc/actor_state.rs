//! Quake edict public fields projected into the shared body physics service.
//!
//! Ported from donor `src/compat/qc/actor-state.ts` (`QcActorState`,
//! `createQcBodyBinding`).
//!
//! Local mirrors (canonical owners live outside this worker's file set, so
//! small structural copies are defined here): [`BodyState`] mirrors
//! `BodyState` from `src/contracts/world.ts`; [`SharedSolid`] and
//! [`SharedPhysicsFlags`] mirror `src/app/bootstrap/simulation/physics.ts`;
//! [`Motion`] mirrors the `Q2Motion` projection consumed by the shared
//! physics service. Entity words are read through
//! [`FieldTable`](crate::fields::FieldTable) instead of the donor
//! `QcMachine`/`QcEntityMemory` pair owned by sibling `qc` workers.

use std::rc::Rc;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Bounds, Vec3, vec3};

use crate::error::GuestError;
use crate::fields::{FieldTable, FieldValue};

/// `FL_FLY`: airborne flight movement.
pub const FLAG_FLY: i32 = 1;
/// `FL_SWIM`: swimming movement.
pub const FLAG_SWIM: i32 = 2;
/// `FL_MONSTER`: monster entity.
pub const FLAG_MONSTER: i32 = 32;
/// `FL_NOTARGET`: ignored by monster targeting and `checkclient`.
pub const FLAG_NOTARGET: i32 = 128;
/// `FL_ITEM`: pickup item (widens link bounds horizontally).
pub const FLAG_ITEM: i32 = 256;
/// `FL_ONGROUND`: standing on ground.
pub const FLAG_ONGROUND: i32 = 512;
/// `FL_PARTIALGROUND`: partially on ground.
pub const FLAG_PARTIALGROUND: i32 = 1024;

/// `solid` 0: non-solid.
pub const SOLID_NOT: f32 = 0.0;
/// `solid` 1: trigger volume.
pub const SOLID_TRIGGER: f32 = 1.0;
/// `solid` 4: brush model.
pub const SOLID_BSP: f32 = 4.0;
/// `solid` 5: corpse (rerelease).
pub const SOLID_CORPSE: f32 = 5.0;

/// Rigid-body snapshot shared with the physics service.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// World-space origin.
    pub origin: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// Local collision bounds.
    pub bounds: Bounds,
    /// Ground actor, if standing on one.
    pub ground: Option<ActorId>,
}

/// Collision family projected from `solid`/`model`/`flags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidKind {
    /// Non-solid.
    None,
    /// Trigger volume.
    Trigger,
    /// Brush model.
    Brush,
    /// Bounding-box solid.
    Box,
}

/// Shared collision record for one actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSolid {
    /// Rerelease corpse that still collides as a brush.
    pub q1_corpse: bool,
    /// Projected solidity.
    pub solid: SolidKind,
    /// Brush model index (`*N` model name, or `0` for the world).
    pub model: Option<i32>,
    /// Owning actor, if the `owner` word is set.
    pub owner: Option<ActorId>,
    /// Monster flag (`FL_MONSTER`).
    pub monster: bool,
    /// Item flag (`FL_ITEM`).
    pub item: bool,
}

/// Motion kind projected from the `movetype` word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// Stationary (`movetype` 0 or 8).
    Stationary,
    /// Step movement (`movetype` 3 or 4).
    Step,
    /// Fly (`movetype` 5).
    Fly,
    /// Toss (`movetype` 6).
    Toss,
    /// Push (`movetype` 7).
    Push,
    /// Fly missile (`movetype` 9).
    FlyMissile,
    /// Bounce (`movetype` 10, or 11 on rerelease builds).
    Bounce,
}

/// Motion record shared with the physics service.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// Projected motion kind.
    pub kind: MotionKind,
    /// Linear velocity, taken from the body snapshot.
    pub velocity: Vec3,
    /// Angular velocity from the `avelocity` word.
    pub angular_velocity: Vec3,
    /// Gravity scale (`gravity` word, defaulting to 1).
    pub gravity: f32,
    /// Gravity direction (always down).
    pub gravity_vector: Vec3,
    /// Collision clip mask.
    pub clip_mask: u32,
    /// Owning actor, shared with the collision record.
    pub owner: Option<ActorId>,
}

/// Physics flags projected from `flags`/`waterlevel`/`watertype`/`health`.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedPhysicsFlags {
    /// Flight flag.
    pub fly: bool,
    /// Swim flag.
    pub swim: bool,
    /// Partial-ground flag.
    pub partial_ground: bool,
    /// Whether the actor is a client.
    pub player: bool,
    /// Water depth level.
    pub water_level: f32,
    /// Water contents type.
    pub water_type: f32,
    /// Whether the actor is dead (`health <= 0`).
    pub dead: bool,
}

/// Partial physics-flag write: only `Some` entries are stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhysicsFlagChanges {
    /// New flight flag.
    pub fly: Option<bool>,
    /// New swim flag.
    pub swim: Option<bool>,
    /// New partial-ground flag.
    pub partial_ground: Option<bool>,
    /// New water level.
    pub water_level: Option<f32>,
    /// New water type.
    pub water_type: Option<f32>,
}

/// Actor lookup services behind [`QcActorState`].
pub trait ActorLookup {
    /// Source slot for an actor, or `None` when unknown.
    fn source_slot(&self, actor: &ActorId) -> Option<u32>;
    /// Resolve an `owner` word to an actor.
    fn reference(&self, reference: i32) -> Option<ActorId>;
    /// Whether the actor is a client.
    fn is_client(&self, actor: &ActorId) -> bool;
}

/// Projects QuakeC entity fields into shared physics records.
#[derive(Debug, Clone)]
pub struct QcActorState<L> {
    lookup: L,
    rerelease: bool,
}

impl<L: ActorLookup> QcActorState<L> {
    /// Build a projector. `rerelease` enables corpse collision and
    /// `movetype` 11, matching the donor `rerelease` option.
    #[must_use]
    pub fn new(lookup: L, rerelease: bool) -> Self {
        Self { lookup, rerelease }
    }

    /// Whether this projector targets a rerelease build.
    #[must_use]
    pub const fn rerelease(&self) -> bool {
        self.rerelease
    }

    /// Project the collision record, or `None` for an unknown actor.
    pub fn collision(&self, fields: &FieldTable, actor: &ActorId) -> Result<Option<SharedSolid>, GuestError> {
        let Some(slot) = self.lookup.source_slot(actor) else {
            return Ok(None);
        };
        let solid = fields.get(actor, "solid")?.as_float("solid")?;
        let flags = fields.get(actor, "flags")?.as_float("flags")? as i32;
        let model_name = fields.get(actor, "model")?.as_text("model")?.to_string();
        let model = if let Some(rest) = model_name.strip_prefix('*') {
            rest.parse::<i32>().ok()
        } else if slot == 0 {
            Some(0)
        } else {
            None
        };
        let corpse = solid == SOLID_CORPSE && self.rerelease;
        let solid_kind = if solid == SOLID_NOT || (solid == SOLID_CORPSE && !corpse) {
            SolidKind::None
        } else if solid == SOLID_TRIGGER {
            SolidKind::Trigger
        } else if solid == SOLID_BSP {
            SolidKind::Brush
        } else {
            SolidKind::Box
        };
        let owner_word = fields.get(actor, "owner")?.as_float("owner")? as i32;
        let owner = if owner_word == 0 {
            None
        } else {
            self.lookup.reference(owner_word)
        };
        Ok(Some(SharedSolid {
            q1_corpse: corpse,
            solid: solid_kind,
            model,
            owner,
            monster: flags & FLAG_MONSTER != 0,
            item: flags & FLAG_ITEM != 0,
        }))
    }

    /// Project the motion record, or `None` for an unknown actor.
    pub fn motion(
        &self,
        fields: &FieldTable,
        actor: &ActorId,
        body: &BodyState,
    ) -> Result<Option<Motion>, GuestError> {
        let Some(_) = self.lookup.source_slot(actor) else {
            return Ok(None);
        };
        let movetype = fields.get(actor, "movetype")?.as_float("movetype")? as i32;
        let kind = match movetype {
            0 | 8 => MotionKind::Stationary,
            3 | 4 => MotionKind::Step,
            5 => MotionKind::Fly,
            6 => MotionKind::Toss,
            7 => MotionKind::Push,
            9 => MotionKind::FlyMissile,
            10 => MotionKind::Bounce,
            11 if self.rerelease => MotionKind::Bounce,
            _ => return Err(GuestError::invalid(format!("Unsupported id1 movetype {movetype}"))),
        };
        let gravity = match fields.get(actor, "gravity") {
            Ok(value) => {
                let gravity = value.as_float("gravity")?;
                if gravity == 0.0 { 1.0 } else { gravity }
            }
            Err(GuestError::UnknownField(_)) => 1.0,
            Err(error) => return Err(error),
        };
        Ok(Some(Motion {
            kind,
            velocity: body.velocity,
            angular_velocity: fields.get(actor, "avelocity")?.as_vector("avelocity")?,
            gravity,
            gravity_vector: vec3(0.0, 0.0, -1.0),
            clip_mask: 3,
            owner: self.collision(fields, actor)?.and_then(|solid| solid.owner),
        }))
    }

    /// Project physics flags; unknown actors read back empty defaults.
    pub fn flags(&self, fields: &FieldTable, actor: &ActorId) -> Result<SharedPhysicsFlags, GuestError> {
        let Some(_) = self.lookup.source_slot(actor) else {
            return Ok(SharedPhysicsFlags {
                fly: false,
                swim: false,
                partial_ground: false,
                player: false,
                water_level: 0.0,
                water_type: 0.0,
                dead: false,
            });
        };
        let flags = fields.get(actor, "flags")?.as_float("flags")? as i32;
        Ok(SharedPhysicsFlags {
            fly: flags & FLAG_FLY != 0,
            swim: flags & FLAG_SWIM != 0,
            partial_ground: flags & FLAG_PARTIALGROUND != 0,
            player: self.lookup.is_client(actor),
            water_level: fields.get(actor, "waterlevel")?.as_float("waterlevel")?,
            water_type: fields.get(actor, "watertype")?.as_float("watertype")?,
            dead: fields.get(actor, "health")?.as_float("health")? <= 0.0,
        })
    }

    /// Store partial physics-flag changes; unknown actors are ignored.
    pub fn write_flags(
        &self,
        fields: &mut FieldTable,
        actor: &ActorId,
        changes: &PhysicsFlagChanges,
    ) -> Result<(), GuestError> {
        let Some(_) = self.lookup.source_slot(actor) else {
            return Ok(());
        };
        let mut flags = fields.get(actor, "flags")?.as_float("flags")? as i32;
        for (value, bit) in [
            (changes.fly, FLAG_FLY),
            (changes.swim, FLAG_SWIM),
            (changes.partial_ground, FLAG_PARTIALGROUND),
        ] {
            if let Some(value) = value {
                flags = if value { flags | bit } else { flags & !bit };
            }
        }
        fields.set(actor, "flags", FieldValue::Float(flags as f32))?;
        if let Some(level) = changes.water_level {
            fields.set(actor, "waterlevel", FieldValue::Float(level))?;
        }
        if let Some(kind) = changes.water_type {
            fields.set(actor, "watertype", FieldValue::Float(kind))?;
        }
        Ok(())
    }

    /// Store angular velocity; unknown actors are ignored.
    pub fn write_angular_velocity(
        &self,
        fields: &mut FieldTable,
        actor: &ActorId,
        value: Vec3,
    ) -> Result<(), GuestError> {
        if self.lookup.source_slot(actor).is_none() {
            return Ok(());
        }
        fields.set(actor, "avelocity", FieldValue::Vector(value))?;
        Ok(())
    }
}

/// Resolves saved ground references back to live actors.
pub type ActorResolver = Rc<dyn Fn(&SavedActorId) -> Option<ActorId>>;

/// Body read/write/link binding over one actor's fields.
pub trait BodyStateBinding {
    /// Read the body snapshot.
    fn read(&self, fields: &FieldTable) -> Result<BodyState, GuestError>;
    /// Write the body snapshot.
    fn write(&self, fields: &mut FieldTable, state: &BodyState) -> Result<(), GuestError>;
    /// Write linked absolute bounds (`absmin`/`absmax`).
    fn linked(&self, fields: &mut FieldTable, absolute: &Bounds) -> Result<(), GuestError>;
}

/// QuakeC body binding created by [`create_qc_body_binding`].
#[derive(Clone)]
pub struct QcBodyBinding {
    actor: ActorId,
    resolve: ActorResolver,
}

impl std::fmt::Debug for QcBodyBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("QcBodyBinding").field("actor", &self.actor).finish()
    }
}

impl QcBodyBinding {
    /// Bound actor.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }
}

impl BodyStateBinding for QcBodyBinding {
    fn read(&self, fields: &FieldTable) -> Result<BodyState, GuestError> {
        let flags = fields.get(&self.actor, "flags")?.as_float("flags")? as i32;
        let ground = if flags & FLAG_ONGROUND == 0 {
            None
        } else {
            fields
                .get(&self.actor, "groundentity")?
                .as_entity("groundentity")?
                .and_then(|saved| (self.resolve)(&saved))
        };
        Ok(BodyState {
            origin: fields.get(&self.actor, "origin")?.as_vector("origin")?,
            angles: fields.get(&self.actor, "angles")?.as_vector("angles")?,
            velocity: fields.get(&self.actor, "velocity")?.as_vector("velocity")?,
            bounds: Bounds {
                min: fields.get(&self.actor, "mins")?.as_vector("mins")?,
                max: fields.get(&self.actor, "maxs")?.as_vector("maxs")?,
            },
            ground,
        })
    }

    fn write(&self, fields: &mut FieldTable, state: &BodyState) -> Result<(), GuestError> {
        fields.set(&self.actor, "origin", FieldValue::Vector(state.origin))?;
        fields.set(&self.actor, "angles", FieldValue::Vector(state.angles))?;
        fields.set(&self.actor, "velocity", FieldValue::Vector(state.velocity))?;
        fields.set(&self.actor, "mins", FieldValue::Vector(state.bounds.min))?;
        fields.set(&self.actor, "maxs", FieldValue::Vector(state.bounds.max))?;
        let flags = fields.get(&self.actor, "flags")?.as_float("flags")? as i32;
        let updated = if state.ground.is_none() { flags & !FLAG_ONGROUND } else { flags | FLAG_ONGROUND };
        fields.set(&self.actor, "flags", FieldValue::Float(updated as f32))?;
        // Source airborne motion clears onground without erasing the previous
        // ground word.
        if let Some(ground) = &state.ground {
            fields.set(
                &self.actor,
                "groundentity",
                FieldValue::Entity(Some(SavedActorId::from(ground))),
            )?;
        }
        Ok(())
    }

    fn linked(&self, fields: &mut FieldTable, absolute: &Bounds) -> Result<(), GuestError> {
        fields.set(&self.actor, "absmin", FieldValue::Vector(absolute.min))?;
        fields.set(&self.actor, "absmax", FieldValue::Vector(absolute.max))?;
        Ok(())
    }
}

/// Create the body binding for one actor. `resolve` maps saved ground
/// references back to live actors.
#[must_use]
pub fn create_qc_body_binding(actor: &ActorId, resolve: ActorResolver) -> QcBodyBinding {
    QcBodyBinding { actor: actor.clone(), resolve }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldLayout;
    use qa_core::identity::IdentityOwner;
    use std::collections::HashMap;

    struct FakeLookup {
        slots: HashMap<ActorId, u32>,
        clients: Vec<ActorId>,
        owners: HashMap<i32, ActorId>,
    }

    impl ActorLookup for FakeLookup {
        fn source_slot(&self, actor: &ActorId) -> Option<u32> {
            self.slots.get(actor).copied()
        }

        fn reference(&self, reference: i32) -> Option<ActorId> {
            self.owners.get(&reference).cloned()
        }

        fn is_client(&self, actor: &ActorId) -> bool {
            self.clients.contains(actor)
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity()
            .field("flags", "float")
            .field("owner", "float")
            .field("movetype", "float")
            .field("avelocity", "vector")
            .field("waterlevel", "float")
            .field("watertype", "float")
            .field("health", "float")
            .field("velocity", "vector")
            .field("mins", "vector")
            .field("maxs", "vector")
            .field("absmin", "vector")
            .field("absmax", "vector")
            .field("groundentity", "entity")
    }

    fn fixture() -> (IdentityOwner, FieldTable) {
        (IdentityOwner::create("actor-state").unwrap(), FieldTable::new())
    }

    #[test]
    fn collision_projects_solid_model_owner_and_item_bits() {
        let (owner, mut fields) = fixture();
        let actor = owner.actor(1, 1);
        let boss = owner.actor(2, 1);
        fields.allocate(&actor, &layout()).unwrap();
        fields.set(&actor, "solid", FieldValue::Float(4.0)).unwrap();
        fields.set(&actor, "model", FieldValue::Text("*2".to_string())).unwrap();
        fields.set(&actor, "flags", FieldValue::Float(288.0)).unwrap();
        fields.set(&actor, "owner", FieldValue::Float(7.0)).unwrap();
        let mut owners = HashMap::new();
        owners.insert(7, boss.clone());
        let mut slots = HashMap::new();
        slots.insert(actor.clone(), 3);
        let state = QcActorState::new(FakeLookup { slots, clients: Vec::new(), owners }, false);
        let solid = state.collision(&fields, &actor).unwrap().unwrap();
        assert!(!solid.q1_corpse);
        assert_eq!(solid.solid, SolidKind::Brush);
        assert_eq!(solid.model, Some(2));
        assert_eq!(solid.owner, Some(boss));
        assert!(solid.monster);
        assert!(solid.item);
    }

    #[test]
    fn collision_treats_world_and_corpses() {
        let (owner, mut fields) = fixture();
        let world = owner.actor(0, 1);
        fields.allocate(&world, &layout()).unwrap();
        fields.set(&world, "solid", FieldValue::Float(5.0)).unwrap();
        fields.set(&world, "model", FieldValue::Text(String::new())).unwrap();
        fields.set(&world, "flags", FieldValue::Float(0.0)).unwrap();
        fields.set(&world, "owner", FieldValue::Float(0.0)).unwrap();
        let mut slots = HashMap::new();
        slots.insert(world.clone(), 0);
        let classic = QcActorState::new(
            FakeLookup { slots: slots.clone(), clients: Vec::new(), owners: HashMap::new() },
            false,
        );
        let solid = classic.collision(&fields, &world).unwrap().unwrap();
        assert_eq!(solid.solid, SolidKind::None);
        assert_eq!(solid.model, Some(0));
        assert!(solid.owner.is_none());
        let rerelease = QcActorState::new(
            FakeLookup { slots, clients: Vec::new(), owners: HashMap::new() },
            true,
        );
        assert!(rerelease.rerelease());
        let solid = rerelease.collision(&fields, &world).unwrap().unwrap();
        assert!(solid.q1_corpse);
        assert_eq!(solid.solid, SolidKind::Box);
    }

    #[test]
    fn unknown_actors_read_empty() {
        let (owner, fields) = fixture();
        let ghost = owner.actor(9, 1);
        let state = QcActorState::new(
            FakeLookup { slots: HashMap::new(), clients: Vec::new(), owners: HashMap::new() },
            false,
        );
        assert!(state.collision(&fields, &ghost).unwrap().is_none());
        let body = BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds { min: vec3(-1.0, -1.0, -1.0), max: vec3(1.0, 1.0, 1.0) },
            ground: None,
        };
        assert!(state.motion(&fields, &ghost, &body).unwrap().is_none());
        let flags = state.flags(&fields, &ghost).unwrap();
        assert!(!flags.player && !flags.dead);
    }

    #[test]
    fn motion_maps_movetypes_and_rejects_unknown() {
        let (owner, mut fields) = fixture();
        let actor = owner.actor(1, 1);
        fields.allocate(&actor, &layout()).unwrap();
        fields.set(&actor, "solid", FieldValue::Float(3.0)).unwrap();
        fields.set(&actor, "model", FieldValue::Text(String::new())).unwrap();
        fields.set(&actor, "flags", FieldValue::Float(0.0)).unwrap();
        fields.set(&actor, "owner", FieldValue::Float(0.0)).unwrap();
        fields.set(&actor, "movetype", FieldValue::Float(7.0)).unwrap();
        fields.set(&actor, "avelocity", FieldValue::Vector(vec3(0.0, 90.0, 0.0))).unwrap();
        let mut slots = HashMap::new();
        slots.insert(actor.clone(), 1);
        let state = QcActorState::new(
            FakeLookup { slots, clients: vec![actor.clone()], owners: HashMap::new() },
            false,
        );
        let body = BodyState {
            origin: vec3(1.0, 2.0, 3.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(4.0, 5.0, 6.0),
            bounds: Bounds { min: vec3(-1.0, -1.0, -1.0), max: vec3(1.0, 1.0, 1.0) },
            ground: None,
        };
        let motion = state.motion(&fields, &actor, &body).unwrap().unwrap();
        assert_eq!(motion.kind, MotionKind::Push);
        assert_eq!(motion.velocity, vec3(4.0, 5.0, 6.0));
        assert_eq!(motion.angular_velocity, vec3(0.0, 90.0, 0.0));
        assert_eq!(motion.gravity, 1.0);
        assert_eq!(motion.gravity_vector, vec3(0.0, 0.0, -1.0));
        assert_eq!(motion.clip_mask, 3);
        fields.set(&actor, "movetype", FieldValue::Float(11.0)).unwrap();
        assert!(state.motion(&fields, &actor, &body).is_err());
    }

    #[test]
    fn flags_round_trip_through_partial_writes() {
        let (owner, mut fields) = fixture();
        let actor = owner.actor(1, 1);
        fields.allocate(&actor, &layout()).unwrap();
        fields.set(&actor, "flags", FieldValue::Float(1025.0)).unwrap();
        fields.set(&actor, "waterlevel", FieldValue::Float(2.0)).unwrap();
        fields.set(&actor, "watertype", FieldValue::Float(-3.0)).unwrap();
        fields.set(&actor, "health", FieldValue::Float(-5.0)).unwrap();
        let mut slots = HashMap::new();
        slots.insert(actor.clone(), 1);
        let state = QcActorState::new(
            FakeLookup { slots, clients: vec![actor.clone()], owners: HashMap::new() },
            false,
        );
        let flags = state.flags(&fields, &actor).unwrap();
        assert!(flags.fly && flags.partial_ground && flags.player && flags.dead);
        assert!(!flags.swim);
        state
            .write_flags(
                &mut fields,
                &actor,
                &PhysicsFlagChanges { fly: Some(false), swim: Some(true), water_level: Some(1.0), ..Default::default() },
            )
            .unwrap();
        let flags = state.flags(&fields, &actor).unwrap();
        assert!(!flags.fly && flags.swim && flags.partial_ground);
        assert_eq!(flags.water_level, 1.0);
        state.write_angular_velocity(&mut fields, &actor, vec3(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(
            fields.get(&actor, "avelocity").unwrap().as_vector("avelocity").unwrap(),
            vec3(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn body_binding_reads_writes_and_links() {
        let (owner, mut fields) = fixture();
        let actor = owner.actor(1, 1);
        let ground = owner.actor(2, 1);
        fields.allocate(&actor, &layout()).unwrap();
        fields.set(&actor, "origin", FieldValue::Vector(vec3(1.0, 2.0, 3.0))).unwrap();
        fields.set(&actor, "flags", FieldValue::Float(512.0)).unwrap();
        fields
            .set(&actor, "groundentity", FieldValue::Entity(Some(SavedActorId::from(&ground))))
            .unwrap();
        let live = ground.clone();
        let binding = create_qc_body_binding(&actor, Rc::new(move |saved: &SavedActorId| {
            (*saved == SavedActorId::from(&live)).then(|| live.clone())
        }));
        assert_eq!(binding.actor(), &actor);
        let body = binding.read(&fields).unwrap();
        assert_eq!(body.origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(body.ground, Some(ground.clone()));
        binding
            .write(
                &mut fields,
                &BodyState {
                    origin: vec3(4.0, 5.0, 6.0),
                    angles: vec3(0.0, 90.0, 0.0),
                    velocity: vec3(0.0, 0.0, -1.0),
                    bounds: Bounds { min: vec3(-16.0, -16.0, -24.0), max: vec3(16.0, 16.0, 32.0) },
                    ground: None,
                },
            )
            .unwrap();
        assert_eq!(
            fields.get(&actor, "flags").unwrap().as_float("flags").unwrap() as i32 & FLAG_ONGROUND,
            0
        );
        // Airborne writes keep the previous ground word.
        assert_eq!(
            fields.get(&actor, "groundentity").unwrap().as_entity("groundentity").unwrap(),
            Some(SavedActorId::from(&ground))
        );
        binding
            .linked(&mut fields, &Bounds { min: vec3(-17.0, -17.0, -25.0), max: vec3(17.0, 17.0, 33.0) })
            .unwrap();
        assert_eq!(
            fields.get(&actor, "absmax").unwrap().as_vector("absmax").unwrap(),
            vec3(17.0, 17.0, 33.0)
        );
    }
}
