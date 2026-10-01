//! Q3 guest slot records binding VM entities to shared actors.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/guest-records.ts`
//! (`Q3GuestRecords`, `Q3GuestRecordHost`).
//!
//! sharedEntity_t borrowing and SV_LinkEntity semantics from id Software's
//! sv_world.c. Copyright (C) 1999-2005 Id Software, Inc. GPL-2.0-or-later.
//!
//! The session surface (actors, bodies, scene, collision) mirrors donor
//! `SessionActorRegistry` (`src/world/actors/registry.ts`), `SharedBodyTable`
//! (`src/world/actors/body.ts`), `SharedSceneQueries`
//! (`src/world/collision/index.ts`), and `ActorCollision`
//! (`src/world/spatial/index.ts`): no Rust home exists yet, so the traits
//! and mirrors below carry the exact donor shapes this module reads
//! (canonical home: the `qa-world` actor/collision port; unify post-merge).
//!
//! Body bindings and the release observer capture the records through a weak
//! handle, so construction returns `Rc<Q3GuestRecords>` via [`Q3GuestRecords::open`]
//! instead of a bare value. Entity mutation goes through guest-memory
//! read-modify-write, matching the donor's live shared-record writes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId, SessionId};
use qa_core::math::{add3, radius_from_bounds, sub3, Bounds, Vec3};
use qa_guest::qvm::game_data::QvmGameData;
use qa_guest::qvm::player_record::QvmPlayerState;
use qa_guest::qvm::shared_entity_record::{write_qvm_shared_entity, QvmEntityCollisionModel, QvmSharedEntity};
use qa_world::body::BodyState;
use qa_world::save::value::{arr, int, obj, SaveJson, SaveReader};
use qa_world::WorldError;

use qa_content::q3::base::world::ActorTraceResult;
use qa_world::movement::types::TraceShape;

use super::guest_world::{Q3GuestBoxLeafnums, Q3GuestTopology};

/// World slot (donor `1022`).
pub const Q3_GUEST_WORLD_SLOT: i32 = 1022;

/// Null slot (donor `1023`).
pub const Q3_GUEST_NULL_SLOT: i32 = 1023;

/// Mirror of donor `ActorCollision["family"]` (`src/world/spatial/index.ts`)
/// (canonical home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GuestCollisionFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Mirror of donor `ActorCollision["shape"]` (canonical home: the
/// `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GuestCollisionShape {
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
    /// Inline brush model.
    Model {
        /// Model index.
        model: i32,
    },
}

/// Mirror of donor `ActorCollision["role"]` (canonical home: the `qa-world`
/// collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GuestCollisionRole {
    /// Solid.
    Solid,
    /// Trigger.
    Trigger,
}

/// Mirror of donor `ActorCollision` (`src/world/spatial/index.ts`)
/// (canonical home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestActorCollision {
    /// Collision family.
    pub family: Q3GuestCollisionFamily,
    /// Collision shape.
    pub shape: Q3GuestCollisionShape,
    /// Contents mask.
    pub contents: i32,
    /// Owning actor.
    pub owner: Option<ActorId>,
    /// Q3 owner numbers.
    pub q3_owner: Option<Q3GuestOwnerNumbers>,
    /// Collision role.
    pub role: Q3GuestCollisionRole,
    /// Monster flag.
    pub monster: bool,
    /// Dead-monster flag.
    pub dead_monster: bool,
}

/// Mirror of donor `ActorCollision["q3Owner"]` (canonical home: the
/// `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3GuestOwnerNumbers {
    /// Entity number.
    pub entity_number: i32,
    /// Owner number.
    pub owner_number: i32,
}

/// Mirror of donor `Q3VisibilityLink` (`src/network/q3/visibility.ts`)
/// (canonical home: the `qa-net` visibility port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestVisibilityLink {
    /// First area number.
    pub areanum: i32,
    /// Second area number.
    pub areanum2: i32,
    /// Visible clusters.
    pub clusters: Vec<i32>,
    /// Last cluster.
    pub last_cluster: i32,
}

/// Mirror of donor `LeafQueryResult` (`src/contracts/scene.ts`) (canonical
/// home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestLeafQuery {
    /// Overlapping leaves.
    pub leaves: Vec<i32>,
    /// Top node.
    pub topnode: Option<i32>,
    /// Overflow flag.
    pub overflow: bool,
}

/// Mirror of donor `SessionActorRegistry.sourceOf` rows
/// (`src/world/actors/registry.ts`) (canonical home: the `qa-world` actor
/// port); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestActorSource {
    /// Owning provider.
    pub provider: ProviderId,
    /// Source slot.
    pub slot: usize,
}

/// Mirror of the donor `SessionActorRegistry` surface this module reads
/// (canonical home: the `qa-world` actor port); unify post-merge.
pub trait Q3GuestRecordActors {
    /// Assert an actor handle is owned.
    fn assert_owned(&self, actor: &OwnedActor);
    /// Allocate an actor at a source slot.
    fn allocate_at_source(&self, provider: &ProviderId, slot: usize, definition: &str) -> OwnedActor;
    /// Source row for an actor.
    fn source_of(&self, actor: &ActorId) -> Option<Q3GuestActorSource>;
    /// Actor at a source slot.
    fn at_source(&self, provider: &ProviderId, slot: usize) -> Option<OwnedActor>;
    /// Actors owned by a provider.
    fn owned_by(&self, provider: &ProviderId) -> Vec<OwnedActor>;
    /// Resolve a saved actor.
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor>;
    /// Observe actor release; returns an unobserve callback.
    fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()>;
    /// Release an actor.
    fn release(&self, actor: &OwnedActor);
    /// Owning session.
    fn session(&self) -> SessionId;
}

/// Live body-state binding (donor `BodyStateBinding`).
pub struct Q3GuestBodyBinding {
    /// Read the body state.
    pub read: Rc<dyn Fn() -> BodyState>,
    /// Write the body state.
    pub write: Rc<dyn Fn(BodyState)>,
}

/// Mirror of the donor `SharedBodyTable` surface this module reads
/// (canonical home: the `qa-world` actor port); unify post-merge.
pub trait Q3GuestRecordBodies {
    /// Bind live body callbacks.
    fn bind(&self, actor: &OwnedActor, binding: Q3GuestBodyBinding);
    /// Unlink a body.
    fn unlink(&self, actor: &OwnedActor);
    /// Link a body.
    fn link(&self, actor: &OwnedActor);
}

/// Mirror of donor `TraceQuery` for world traces (`src/contracts/scene.ts`)
/// (canonical home: the `qa-world` collision port); unify post-merge.
/// The target is always the world and the numeric profile is always the Q3
/// binary32 profile, matching the donor call sites.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GuestTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
    /// Curve collision flag.
    pub curves: bool,
    /// Player curve-clip flag.
    pub player_curve_clip: bool,
}

/// Mirror of donor `TraceQuery["target"]` for point contents
/// (canonical home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q3GuestPointTarget {
    /// World target.
    World,
    /// Brush-model target.
    Model {
        /// Model index.
        model: i32,
        /// Model origin.
        origin: Vec3,
        /// Model angles.
        angles: Vec3,
    },
}

/// Mirror of donor `TraceQuery` for brush-model contact traces
/// (canonical home: the `qa-world` collision port); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GuestModelTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Model index.
    pub model: i32,
    /// Model origin.
    pub origin: Vec3,
    /// Model angles.
    pub angles: Vec3,
    /// Contents mask.
    pub mask: i32,
    /// Curve collision flag.
    pub curves: bool,
    /// Player curve-clip flag.
    pub player_curve_clip: bool,
}

/// Actor-collision reader callback.
pub type Q3GuestCollisionReader = Rc<dyn Fn(&ActorId) -> Option<Q3GuestActorCollision>>;
/// Actor-collision reporter callback.
pub type Q3GuestCollisionReporter = Rc<dyn Fn(&OwnedActor, &Q3GuestActorCollision)>;
/// Actor admission callback.
pub type Q3GuestAdmission = Rc<dyn Fn(&OwnedActor)>;

/// Mirror of the donor `SharedSceneQueries` surface the guest reads
/// (canonical home: the `qa-world` collision port); unify post-merge.
///
/// One trait serves records and spatial queries because the donor enforces
/// that both use the same scene object.
pub trait Q3GuestScene: Q3GuestTopology {
    /// Bind the actor-collision reader.
    fn bind_actor_collision(&self, read: Q3GuestCollisionReader);
    /// Leaves overlapping bounds, up to `limit`.
    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> Q3GuestLeafQuery;
    /// Cluster of a leaf.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Area of a leaf.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Trace against the world with actor collision.
    fn trace(&self, query: &Q3GuestTraceQuery) -> ActorTraceResult;
    /// Q3 contents at a point against the world or a brush model.
    fn point_contents(&self, point: Vec3, target: &Q3GuestPointTarget, pass_actor: Option<&ActorId>) -> i32;
    /// Actors touching bounds.
    fn query_actors(&self, bounds: &Bounds) -> Vec<ActorId>;
    /// Trace a shape against a brush model.
    fn geometry_trace(&self, query: &Q3GuestModelTraceQuery) -> ActorTraceResult;
    /// Bounds of a brush model.
    fn model_bounds(&self, model: i32) -> Bounds;
    /// Whether two areas are connected.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Leaf containing a point.
    fn point_leaf(&self, point: Vec3) -> i32;
    /// PVS visibility between clusters.
    fn cluster_visible(&self, from: i32, cluster: i32) -> bool;
}

/// Session host behind guest records, mirroring donor `Q3GuestRecordHost`.
pub struct Q3GuestRecordHost {
    /// Session actors.
    pub actors: Rc<dyn Q3GuestRecordActors>,
    /// Shared bodies.
    pub bodies: Rc<dyn Q3GuestRecordBodies>,
    /// Shared scene.
    pub scene: Rc<dyn Q3GuestScene>,
    /// Guest provider.
    pub provider: ProviderId,
    /// Report an actor collision.
    pub collision: Q3GuestCollisionReporter,
    /// Admit an actor, if the session filters admissions.
    pub admit: Option<Q3GuestAdmission>,
}

/// Player-velocity writer (donor `setPlayerVelocityWriter` hook).
pub type Q3GuestVelocityWriter = Rc<dyn Fn(&OwnedActor, Vec3)>;

/// Identities denote VM slots. The ABI exposes unlink, but no game-private
/// free event.
pub struct Q3GuestRecords {
    /// Game data (shared handle; clones observe the same tables).
    pub data: QvmGameData,
    /// Session host.
    pub host: Q3GuestRecordHost,
    me: Weak<Q3GuestRecords>,
    write_player_velocity: RefCell<Option<Q3GuestVelocityWriter>>,
    actors: RefCell<HashMap<i32, OwnedActor>>,
    links: RefCell<HashMap<i32, Q3GuestVisibilityLink>>,
    retired_inputs: RefCell<HashSet<i32>>,
    input_motion: RefCell<HashSet<i32>>,
    closed: RefCell<bool>,
    unobserve: RefCell<Option<Box<dyn Fn()>>>,
}

impl Q3GuestRecords {
    /// Open guest records over shared game data.
    #[must_use]
    pub fn open(data: QvmGameData, host: Q3GuestRecordHost) -> Rc<Self> {
        Rc::new_cyclic(|me: &Weak<Self>| {
            let collision_me = me.clone();
            host.scene.bind_actor_collision(Rc::new(move |actor| {
                collision_me.upgrade().and_then(|records| {
                    let slot = records.slot(actor)?;
                    if slot < 0 || slot as usize >= records.data.num_entities() {
                        return None;
                    }
                    Some(records.collision(slot))
                })
            }));
            let release_me = me.clone();
            let unobserve = host.actors.on_release(Box::new(move |actor| {
                if let Some(records) = release_me.upgrade() {
                    let slot = records
                        .actors
                        .borrow()
                        .iter()
                        .find(|(_, owned)| owned.id() == actor.id())
                        .map(|(slot, _)| *slot);
                    if let Some(slot) = slot {
                        records.actors.borrow_mut().remove(&slot);
                        records.links.borrow_mut().remove(&slot);
                    }
                }
            }));
            Self {
                data,
                host,
                me: me.clone(),
                write_player_velocity: RefCell::new(None),
                actors: RefCell::new(HashMap::new()),
                links: RefCell::new(HashMap::new()),
                retired_inputs: RefCell::new(HashSet::new()),
                input_motion: RefCell::new(HashSet::new()),
                closed: RefCell::new(false),
                unobserve: RefCell::new(Some(unobserve)),
            }
        })
    }

    /// Install the player-velocity writer.
    pub fn set_player_velocity_writer(&self, write: Q3GuestVelocityWriter) {
        *self.write_player_velocity.borrow_mut() = Some(write);
    }

    fn check_open(&self) {
        if *self.closed.borrow() {
            panic!("Q3 guest records are retired");
        }
    }

    fn read_entity(&self, slot: i32) -> QvmSharedEntity {
        self.check_open();
        let number = usize::try_from(slot).unwrap_or_else(|_| panic!("Q3 guest slot {slot} is invalid"));
        self.data
            .entity(number)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"))
    }

    /// Write a shared entity record back to guest memory.
    ///
    /// The donor mutates live shared records; the Rust port reads owned
    /// copies through [`Q3GuestRecords::entity`], so explicit write-back
    /// closes the read-modify-write loop.
    pub fn write_entity(&self, slot: i32, entity: &QvmSharedEntity) {
        let number = usize::try_from(slot).unwrap_or_else(|_| panic!("Q3 guest slot {slot} is invalid"));
        let window = self
            .data
            .public_entity_bytes(number)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
        let mut bytes = window
            .copy_bytes(0, window.len)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
        write_qvm_shared_entity(&mut bytes, entity, self.data.abi_profile())
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
        window
            .memory
            .write_bytes(window.offset, &bytes)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
    }

    /// Read a shared entity record.
    #[must_use]
    pub fn entity(&self, slot: i32) -> QvmSharedEntity {
        self.read_entity(slot)
    }

    /// Copy a client player state.
    #[must_use]
    pub fn player(&self, slot: i32) -> QvmPlayerState {
        self.check_open();
        let number = usize::try_from(slot).unwrap_or_else(|_| panic!("Q3 guest slot {slot} is invalid"));
        self.data
            .copy_player_state(number)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"))
    }

    /// Write a client view roll for the current client actor.
    pub fn set_player_view_roll(&self, actor: &ActorId, roll: f64) {
        let slot = self.slot(actor);
        if slot.is_none_or(|slot| {
            slot < 0 || slot as usize >= self.data.num_clients() || self.retired_inputs.borrow().contains(&slot)
        }) {
            panic!("Q3 source view requires the current client actor");
        }
        let slot = slot.expect("checked slot");
        let mut state = self.player(slot);
        state.view_angles.z = roll as f32;
        let number = usize::try_from(slot).expect("checked slot");
        self.data
            .write_player_state(number, &state)
            .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
    }

    /// Actor owning a VM slot, allocating at the source on first use.
    #[must_use]
    pub fn actor(&self, slot: i32) -> OwnedActor {
        self.read_entity(slot);
        if self.retired_inputs.borrow().contains(&slot) {
            panic!("Q3 input client is retiring");
        }
        if slot >= Q3_GUEST_WORLD_SLOT {
            panic!("Q3 world and none slots cannot own shared actors");
        }
        if let Some(current) = self.actors.borrow().get(&slot) {
            self.host.actors.assert_owned(current);
            return current.clone();
        }
        let actor = self
            .host
            .actors
            .allocate_at_source(&self.host.provider, slot as usize, "q3:guest-slot");
        self.bind(slot, &actor);
        actor
    }

    fn bind(&self, slot: i32, actor: &OwnedActor) {
        self.actors.borrow_mut().insert(slot, actor.clone());
        let me = self.me.clone();
        let read = Rc::new(move || me.upgrade().expect("Q3 guest records are retired").body(slot));
        let me = self.me.clone();
        let owned = actor.clone();
        let write = Rc::new(move |body: BodyState| {
            let records = me.upgrade().expect("Q3 guest records are retired");
            records.write_body(slot, &owned, &body);
        });
        self.host.bodies.bind(actor, Q3GuestBodyBinding { read, write });
        if let Some(admit) = &self.host.admit {
            admit(actor);
        }
    }

    fn write_body(&self, slot: i32, actor: &OwnedActor, body: &BodyState) {
        if self.input_motion.borrow().contains(&slot) {
            let mut state = self.player(slot);
            state.origin = body.origin;
            state.velocity = body.velocity;
            let number = usize::try_from(slot).expect("guest slot");
            self.data
                .write_player_state(number, &state)
                .unwrap_or_else(|error| panic!("Q3 guest slot {slot}: {error}"));
        } else if (slot as usize) < self.data.num_clients() {
            if let Some(write) = self.write_player_velocity.borrow().clone() {
                write(actor, body.velocity);
            }
        }
        let mut entity = self.read_entity(slot);
        entity.r.current_origin = body.origin;
        entity.r.current_angles = body.angles;
        entity.r.mins = body.bounds.min;
        entity.r.maxs = body.bounds.max;
        entity.s.pos.delta = body.velocity;
        if !self.input_motion.borrow().contains(&slot) {
            entity.s.ground_entity_num = body
                .ground
                .as_ref()
                .map_or(Q3_GUEST_NULL_SLOT, |ground| self.require_slot(ground));
        }
        self.write_entity(slot, &entity);
    }

    /// VM slot for a shared actor, if bound.
    #[must_use]
    pub fn slot(&self, actor: &ActorId) -> Option<i32> {
        if *self.closed.borrow() {
            return None;
        }
        let source = self.host.actors.source_of(actor)?;
        if source.provider != self.host.provider {
            return None;
        }
        let slot = i32::try_from(source.slot).ok()?;
        if self.actors.borrow().get(&slot).is_some_and(|owned| owned.id() == actor) {
            Some(slot)
        } else {
            None
        }
    }

    /// VM slot for a shared actor, panicking when unbound.
    #[must_use]
    pub fn require_slot(&self, actor: &ActorId) -> i32 {
        self.slot(actor)
            .unwrap_or_else(|| panic!("Shared actor has no Q3 guest slot"))
    }

    /// Shared actor for a VM slot, if live.
    #[must_use]
    pub fn reference(&self, slot: i32) -> Option<ActorId> {
        if !(0..Q3_GUEST_WORLD_SLOT).contains(&slot) || self.retired_inputs.borrow().contains(&slot) {
            return None;
        }
        Some(self.actor(slot).id().clone())
    }

    /// Body state for a VM slot.
    #[must_use]
    pub fn body(&self, slot: i32) -> BodyState {
        let entity = self.read_entity(slot);
        let shared = &entity.r;
        BodyState {
            origin: if self.input_motion.borrow().contains(&slot) {
                self.player(slot).origin
            } else {
                shared.current_origin
            },
            angles: shared.current_angles,
            velocity: if (slot as usize) < self.data.num_clients() {
                self.player(slot).velocity
            } else {
                entity.s.pos.delta
            },
            bounds: qa_core::math::Bounds {
                min: shared.mins,
                max: shared.maxs,
            },
            ground: self
                .actors
                .borrow()
                .get(&entity.s.ground_entity_num)
                .map(|actor| actor.id().clone()),
        }
    }

    /// Collision record for a VM slot.
    #[must_use]
    pub fn collision(&self, slot: i32) -> Q3GuestActorCollision {
        let entity = self.read_entity(slot);
        let shared = &entity.r;
        Q3GuestActorCollision {
            family: Q3GuestCollisionFamily::Q3,
            shape: match shared.model {
                QvmEntityCollisionModel::Inline { index } => Q3GuestCollisionShape::Model { model: index },
                QvmEntityCollisionModel::Box => Q3GuestCollisionShape::Box,
                QvmEntityCollisionModel::Capsule => Q3GuestCollisionShape::Capsule,
            },
            contents: shared.contents,
            owner: self
                .actors
                .borrow()
                .get(&shared.owner_num)
                .map(|actor| actor.id().clone()),
            q3_owner: Some(Q3GuestOwnerNumbers {
                entity_number: slot,
                owner_number: shared.owner_num,
            }),
            role: Q3GuestCollisionRole::Solid,
            monster: false,
            dead_monster: false,
        }
    }

    /// Link a VM slot into collision (donor `SV_LinkEntity` port).
    pub fn link(&self, slot: i32) {
        if self.retired_inputs.borrow().contains(&slot) {
            let mut entity = self.read_entity(slot);
            entity.r.linked = false;
            self.write_entity(slot, &entity);
            return;
        }
        let mut entity = self.read_entity(slot);
        let actor = self.actor(slot);
        self.host.bodies.unlink(&actor);
        entity.r.linked = false;
        let byte = |value: f32| {
            let truncated = value.trunc();
            if truncated.is_nan() {
                0
            } else {
                truncated.clamp(1.0, 255.0) as i32
            }
        };
        entity.s.solid = match entity.r.model {
            QvmEntityCollisionModel::Inline { .. } => 0x00ff_ffff,
            _ if entity.r.contents & (1 | 0x200_0000) == 0 => 0,
            _ => (byte(entity.r.maxs.z + 32.0) << 16) | (byte(-entity.r.mins.z) << 8) | byte(entity.r.maxs.x),
        };
        let angles = entity.r.current_angles;
        let origin = entity.r.current_origin;
        let rotated = matches!(entity.r.model, QvmEntityCollisionModel::Inline { .. })
            && (angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0);
        let radius = if rotated {
            radius_from_bounds(Bounds {
                min: entity.r.mins,
                max: entity.r.maxs,
            })
        } else {
            0.0
        };
        let extent = Vec3 {
            x: radius,
            y: radius,
            z: radius,
        };
        let epsilon = Vec3 { x: 1.0, y: 1.0, z: 1.0 };
        entity.r.absmin = sub3(
            if rotated {
                sub3(origin, extent)
            } else {
                add3(origin, entity.r.mins)
            },
            epsilon,
        );
        entity.r.absmax = add3(
            if rotated {
                add3(origin, extent)
            } else {
                add3(origin, entity.r.maxs)
            },
            epsilon,
        );
        self.write_entity(slot, &entity);
        let models = self.host.scene.native_q3_clip_models();
        let bounds = Bounds {
            min: entity.r.absmin,
            max: entity.r.absmax,
        };
        let native_leaves: Option<Q3GuestBoxLeafnums> = models.as_ref().map(|models| models.box_leafnums(&bounds, 128));
        let leaves: Vec<i32> = native_leaves.as_ref().map_or_else(
            || {
                self.host
                    .scene
                    .box_leaves(&bounds, self.host.scene.geometry().leaf_count)
                    .leaves
            },
            |native| native.leaves.clone(),
        );
        let mut cluster_leaves = leaves.clone();
        if native_leaves.is_none() {
            cluster_leaves.sort_by_key(|leaf| self.host.scene.leaf_cluster(*leaf));
        }
        let last_leaf = native_leaves.as_ref().map_or_else(
            || cluster_leaves.last().copied().unwrap_or(0),
            |native| native.last_leaf,
        );
        let mut areanum = -1;
        let mut areanum2 = -1;
        let mut last_cluster = 0;
        let mut clusters = Vec::new();
        for leaf in &leaves {
            let area = self.host.scene.leaf_area(*leaf);
            if area == -1 {
                continue;
            }
            if areanum != -1 && areanum != area {
                areanum2 = area;
            } else {
                areanum = area;
            }
        }
        for leaf in &cluster_leaves {
            let cluster = self.host.scene.leaf_cluster(*leaf);
            if cluster == -1 {
                continue;
            }
            clusters.push(cluster);
            if clusters.len() == 16 {
                last_cluster = self.host.scene.leaf_cluster(last_leaf);
                break;
            }
        }
        self.links.borrow_mut().insert(
            slot,
            Q3GuestVisibilityLink {
                areanum,
                areanum2,
                clusters,
                last_cluster,
            },
        );
        if leaves.is_empty() {
            return;
        }
        let collision = self.collision(slot);
        (self.host.collision)(&actor, &collision);
        self.host.bodies.link(&actor);
        let mut entity = self.read_entity(slot);
        entity.r.linkcount = entity.r.linkcount.wrapping_add(1);
        entity.r.linked = true;
        self.write_entity(slot, &entity);
    }

    /// Unlink a VM slot from collision.
    pub fn unlink(&self, slot: i32) {
        let mut entity = self.read_entity(slot);
        entity.r.linked = false;
        self.write_entity(slot, &entity);
        if let Some(actor) = self.actors.borrow().get(&slot).cloned() {
            self.host.bodies.unlink(&actor);
        }
    }

    /// Visibility link for a VM slot, if linked.
    #[must_use]
    pub fn visibility(&self, slot: i32) -> Option<Q3GuestVisibilityLink> {
        self.read_entity(slot);
        self.links.borrow().get(&slot).cloned()
    }

    /// Capture the actor/link checkpoint value.
    #[must_use]
    pub fn capture_checkpoint(&self) -> SaveJson {
        if *self.closed.borrow() {
            panic!("Q3 guest records are retired");
        }
        if !self.retired_inputs.borrow().is_empty() || !self.input_motion.borrow().is_empty() {
            panic!("Q3 input must finish before checkpoint");
        }
        let mut entries: Vec<(i32, OwnedActor)> = self
            .actors
            .borrow()
            .iter()
            .map(|(slot, actor)| (*slot, actor.clone()))
            .collect();
        entries.sort_by_key(|(slot, _)| *slot);
        arr(entries
            .into_iter()
            .map(|(slot, actor)| {
                self.host.actors.assert_owned(&actor);
                let link = self.links.borrow().get(&slot).cloned();
                obj(vec![
                    ("slot", int(i64::from(slot))),
                    (
                        "actor",
                        obj(vec![
                            ("slot", int(i64::from(SavedActorId::from(actor.id()).slot))),
                            ("generation", int(i64::from(SavedActorId::from(actor.id()).generation))),
                        ]),
                    ),
                    (
                        "visibility",
                        link.map_or(SaveJson::Null, |link| {
                            obj(vec![
                                ("areanum", int(i64::from(link.areanum))),
                                ("areanum2", int(i64::from(link.areanum2))),
                                (
                                    "clusters",
                                    arr(link
                                        .clusters
                                        .into_iter()
                                        .map(|cluster| int(i64::from(cluster)))
                                        .collect()),
                                ),
                                ("lastCluster", int(i64::from(link.last_cluster))),
                            ])
                        }),
                    ),
                ])
            })
            .collect())
    }

    /// Restore actor/link bindings into a fresh candidate.
    pub fn restore_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError> {
        if *self.closed.borrow() || !self.actors.borrow().is_empty() {
            panic!("Q3 guest records restore requires a fresh candidate");
        }
        let reader = SaveReader::at(value, "q3.guest.records");
        let entries = reader.list(|entry| {
            let actor = entry.field("actor");
            let slot_word = u32::try_from(actor.field("slot").integer(0)?)
                .map_err(|_| entry.fail("invalid source actor or visibility binding"))?;
            let generation_word = u32::try_from(actor.field("generation").integer(0)?)
                .map_err(|_| entry.fail("invalid source actor or visibility binding"))?;
            let visibility = entry.field("visibility").nullable(|link| {
                let clusters = link.field("clusters").list(|cluster| cluster.integer(0))?;
                let mut checked = Vec::with_capacity(clusters.len());
                for cluster in clusters {
                    checked.push(
                        i32::try_from(cluster).map_err(|_| link.fail("invalid source actor or visibility binding"))?,
                    );
                }
                let areanum = i32::try_from(link.field("areanum").integer(-1)?)
                    .map_err(|_| link.fail("invalid source actor or visibility binding"))?;
                let areanum2 = i32::try_from(link.field("areanum2").integer(-1)?)
                    .map_err(|_| link.fail("invalid source actor or visibility binding"))?;
                let last_cluster = i32::try_from(link.field("lastCluster").integer(-1)?)
                    .map_err(|_| link.fail("invalid source actor or visibility binding"))?;
                Ok::<_, WorldError>(Q3GuestVisibilityLink {
                    areanum,
                    areanum2,
                    clusters: checked,
                    last_cluster,
                })
            })?;
            Ok::<_, WorldError>((
                entry.field("slot").integer(0)?,
                SavedActorId {
                    slot: slot_word,
                    generation: generation_word,
                },
                visibility,
            ))
        })?;
        let mut slots = HashSet::new();
        let mut actors = HashSet::new();
        let mut bindings = Vec::new();
        for (slot, saved, visibility) in entries {
            let slot = i32::try_from(slot).map_err(|_| reader.fail("invalid source actor or visibility binding"))?;
            self.read_entity(slot);
            let actor = self.host.actors.resolve_saved(&saved);
            let valid = slot < Q3_GUEST_WORLD_SLOT
                && slots.insert(slot)
                && actor.as_ref().is_some_and(|actor| actors.insert(actor.id().clone()))
                && actor.as_ref().is_some_and(|actor| actor.owner() == &self.host.provider)
                && actor.as_ref().is_some_and(|actor| {
                    self.host
                        .actors
                        .at_source(&self.host.provider, slot as usize)
                        .is_some_and(|at| at.id() == actor.id())
                })
                && visibility.as_ref().is_none_or(|link| link.clusters.len() <= 16);
            if !valid {
                return Err(reader.fail("invalid source actor or visibility binding"));
            }
            bindings.push((slot, actor.expect("validated actor"), visibility));
        }
        let owned = self
            .host
            .actors
            .owned_by(&self.host.provider)
            .iter()
            .filter(|actor| {
                self.host
                    .actors
                    .source_of(actor.id())
                    .is_some_and(|source| source.slot as i32 != Q3_GUEST_WORLD_SLOT)
            })
            .count();
        if actors.len() != owned {
            return Err(reader.fail("guest record checkpoint omits owned actors"));
        }
        for (slot, actor, visibility) in bindings {
            self.bind(slot, &actor);
            if let Some(link) = visibility {
                self.links.borrow_mut().insert(slot, link);
            }
        }
        Ok(())
    }

    /// Release a client slot and its actor.
    pub fn release_client(&self, slot: i32) {
        if slot < 0 || slot as usize >= self.data.num_clients() {
            panic!("Q3 client slot is outside configured game data");
        }
        if (slot as usize) < self.data.num_entities() {
            self.unlink(slot);
        }
        if let Some(actor) = self.actors.borrow().get(&slot).cloned() {
            self.host.actors.release(&actor);
        }
    }

    /// Retire an input client whose actor is being removed.
    pub fn retire_input_client(&self, slot: i32) {
        self.retired_inputs.borrow_mut().insert(slot);
        self.unlink(slot);
    }

    /// Finish an input retirement.
    pub fn finish_input_retirement(&self, slot: i32) {
        self.retired_inputs.borrow_mut().remove(&slot);
    }

    /// Whether an input client is retired.
    #[must_use]
    pub fn is_input_retired(&self, slot: i32) -> bool {
        self.retired_inputs.borrow().contains(&slot)
    }

    /// Run a syscall with input-motion body reads for a slot.
    pub fn with_input_motion(&self, slot: i32, run: impl FnOnce() -> i32) -> i32 {
        struct Guard<'a> {
            records: &'a Q3GuestRecords,
            slot: i32,
            previous: bool,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if !self.previous {
                    self.records.input_motion.borrow_mut().remove(&self.slot);
                }
            }
        }
        let previous = self.input_motion.borrow().contains(&slot);
        self.input_motion.borrow_mut().insert(slot);
        let guard = Guard {
            records: self,
            slot,
            previous,
        };
        let result = run();
        drop(guard);
        result
    }

    /// Release every actor and retire the records.
    pub fn close(&self) {
        if *self.closed.borrow() {
            return;
        }
        let actors: Vec<(i32, OwnedActor)> = self
            .actors
            .borrow()
            .iter()
            .map(|(slot, actor)| (*slot, actor.clone()))
            .collect();
        for (slot, actor) in actors {
            if (slot as usize) < self.data.num_entities() {
                let mut entity = self.read_entity(slot);
                entity.r.linked = false;
                self.write_entity(slot, &entity);
            }
            self.host.actors.release(&actor);
        }
        *self.closed.borrow_mut() = true;
        if let Some(unobserve) = self.unobserve.borrow_mut().take() {
            unobserve();
        }
        self.links.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Bounds, Vec3};
    use qa_guest::qvm::game_data::{AbiProfile, QvmSharedMemory};

    use super::super::guest_world::{Q3GuestGeometry, Q3GuestGeometryKind, Q3GuestNativeClipWorld};
    use super::*;

    type Watcher = Box<dyn Fn(&OwnedActor)>;

    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        sources: RefCell<HashMap<ActorId, Q3GuestActorSource>>,
        watchers: RefCell<Vec<Watcher>>,
    }

    impl FakeActors {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("q3-guest-records").unwrap(),
                owned: RefCell::new(HashMap::new()),
                sources: RefCell::new(HashMap::new()),
                watchers: RefCell::new(Vec::new()),
            }
        }
    }

    impl Q3GuestRecordActors for FakeActors {
        fn assert_owned(&self, actor: &OwnedActor) {
            assert!(self.owner.owns_owned(actor));
        }

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let id = self.owner.actor(slot as u32, 1);
            let owned = self.owner.owned_actor(&id, provider.clone()).unwrap();
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.sources.borrow_mut().insert(
                id,
                Q3GuestActorSource {
                    provider: provider.clone(),
                    slot,
                },
            );
            owned
        }

        fn source_of(&self, actor: &ActorId) -> Option<Q3GuestActorSource> {
            self.sources.borrow().get(actor).cloned()
        }

        fn at_source(&self, provider: &ProviderId, slot: usize) -> Option<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .find(|actor| {
                    actor.owner() == provider
                        && self
                            .sources
                            .borrow()
                            .get(actor.id())
                            .is_some_and(|source| source.slot == slot)
                })
                .cloned()
        }

        fn owned_by(&self, provider: &ProviderId) -> Vec<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .filter(|actor| actor.owner() == provider)
                .cloned()
                .collect()
        }

        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .find(|actor| SavedActorId::from(actor.id()) == *saved)
                .cloned()
        }

        fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            self.watchers.borrow_mut().push(callback);
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.owned.borrow_mut().remove(actor.id());
            self.sources.borrow_mut().remove(actor.id());
        }

        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
    }

    struct FakeBodies {
        bindings: RefCell<HashMap<ActorId, Q3GuestBodyBinding>>,
        linked: RefCell<HashSet<ActorId>>,
    }

    impl Q3GuestRecordBodies for FakeBodies {
        fn bind(&self, actor: &OwnedActor, binding: Q3GuestBodyBinding) {
            self.bindings.borrow_mut().insert(actor.id().clone(), binding);
        }

        fn unlink(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().remove(actor.id());
        }

        fn link(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor.id().clone());
        }
    }

    struct FakeScene;

    impl Q3GuestTopology for FakeScene {
        fn geometry(&self) -> Q3GuestGeometry {
            Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q3Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 1,
            }
        }

        fn adjust_area_portal_state(&self, _first: i32, _second: i32, _open: bool) {}

        fn adjust_area_portal_contribution(&self, _portal: i32, _delta: i32) {}

        fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
            None
        }
    }

    impl Q3GuestScene for FakeScene {
        fn bind_actor_collision(&self, _read: Rc<dyn Fn(&ActorId) -> Option<Q3GuestActorCollision>>) {}

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
            Q3GuestLeafQuery {
                leaves: vec![0],
                topnode: None,
                overflow: false,
            }
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn trace(&self, _query: &Q3GuestTraceQuery) -> ActorTraceResult {
            panic!("unused in records tests");
        }

        fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
            panic!("unused in records tests");
        }

        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            panic!("unused in records tests");
        }

        fn geometry_trace(&self, _query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
            panic!("unused in records tests");
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            panic!("unused in records tests");
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            panic!("unused in records tests");
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            panic!("unused in records tests");
        }

        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            panic!("unused in records tests");
        }
    }

    fn data() -> QvmGameData {
        let memory = QvmSharedMemory::new(65536).unwrap();
        let data = QvmGameData::new(memory, AbiProfile::Modern);
        data.set_client_count(2).unwrap();
        data.locate(64, 8, 560, 8192, 560).unwrap();
        data
    }

    fn host() -> (Rc<FakeActors>, Rc<FakeBodies>, Q3GuestRecordHost) {
        let actors = Rc::new(FakeActors::new());
        let bodies = Rc::new(FakeBodies {
            bindings: RefCell::new(HashMap::new()),
            linked: RefCell::new(HashSet::new()),
        });
        let host = Q3GuestRecordHost {
            actors: actors.clone(),
            bodies: bodies.clone(),
            scene: Rc::new(FakeScene),
            provider: ProviderId::new("q3", "guest-test"),
            collision: Rc::new(|_, _| {}),
            admit: None,
        };
        (actors, bodies, host)
    }

    #[test]
    fn slots_bind_and_resolve_actors() {
        let (_actors, bodies, host) = host();
        let records = Q3GuestRecords::open(data(), host);
        let actor = records.actor(0);
        assert_eq!(records.slot(actor.id()), Some(0));
        assert_eq!(records.reference(0), Some(actor.id().clone()));
        assert_eq!(records.reference(1022), None);
        assert!(bodies.bindings.borrow().contains_key(actor.id()));
        let body = records.body(0);
        assert_eq!(body.origin, vec3(0.0, 0.0, 0.0));
        let collision = records.collision(0);
        assert_eq!(collision.family, Q3GuestCollisionFamily::Q3);
        assert_eq!(collision.role, Q3GuestCollisionRole::Solid);
    }

    #[test]
    fn link_computes_visibility_and_solid() {
        let (_actors, bodies, host) = host();
        let records = Q3GuestRecords::open(data(), host);
        let _ = records.actor(1);
        records.link(1);
        let entity = records.entity(1);
        assert!(entity.r.linked);
        assert_eq!(entity.r.linkcount, 1);
        let link = records.visibility(1).unwrap();
        assert_eq!((link.areanum, link.areanum2), (0, -1));
        assert_eq!(link.clusters, vec![0]);
        let actor = records.actor(1);
        assert!(bodies.linked.borrow().contains(actor.id()));
        records.unlink(1);
        assert!(!records.entity(1).r.linked);
        assert!(!bodies.linked.borrow().contains(actor.id()));
    }

    #[test]
    fn view_roll_and_input_motion() {
        let (_actors, _bodies, host) = host();
        let records = Q3GuestRecords::open(data(), host);
        let actor = records.actor(0);
        records.set_player_view_roll(actor.id(), 45.0);
        assert_eq!(records.player(0).view_angles.z, 45.0);
        let seen = records.with_input_motion(0, || 7);
        assert_eq!(seen, 7);
        assert!(!records.input_motion.borrow().contains(&0));
        records.retire_input_client(0);
        assert!(records.is_input_retired(0));
        assert_eq!(records.reference(0), None);
        records.finish_input_retirement(0);
        assert!(!records.is_input_retired(0));
    }

    #[test]
    fn checkpoint_round_trip() {
        let (actors, _bodies, host) = host();
        let provider = ProviderId::new("q3", "guest-test");
        let records = Q3GuestRecords::open(data(), host);
        let _ = records.actor(0);
        let _ = records.actor(1);
        records.link(0);
        let image = records.capture_checkpoint();
        let fresh = Q3GuestRecords::open(
            data(),
            Q3GuestRecordHost {
                actors: actors.clone(),
                bodies: Rc::new(FakeBodies {
                    bindings: RefCell::new(HashMap::new()),
                    linked: RefCell::new(HashSet::new()),
                }),
                scene: Rc::new(FakeScene),
                provider,
                collision: Rc::new(|_, _| {}),
                admit: None,
            },
        );
        fresh.restore_checkpoint(&image).unwrap();
        assert_eq!(fresh.slot(&fresh.actor(0).id().clone()), Some(0));
        assert!(fresh.visibility(0).is_some());
        assert!(fresh.visibility(1).is_none());
        fresh.close();
    }

    #[test]
    fn release_and_close_retire_slots() {
        let (_actors, _bodies, host) = host();
        let records = Q3GuestRecords::open(data(), host);
        let actor = records.actor(0);
        records.release_client(0);
        assert_eq!(records.slot(actor.id()), None);
        records.close();
    }
}
