//! Shared body physics for every source family.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/physics.ts`
//! (`SharedPhysics`; adapted from Quake `sv_phys.c`/`sv_move.c` and Quake II
//! `g_phys.c`/`m_move.c`).
//!
//! The engine tables behind this owner — the actor registry, body table,
//! scene queries, and touch callbacks — live outside the simulation lane, so
//! they surface as the [`PhysicsActors`], [`PhysicsBodies`], [`PhysicsScene`],
//! and [`PhysicsCallbacks`] seams. Only the operations the donor calls are
//! modeled. The trace vocabulary reuses the real per-family traces
//! ([`Q1Trace`], [`Q2Trace`], [`Q3Trace`]) under one [`TraceResult`] union.

use std::cmp::Ordering;
use std::collections::HashMap;

use qa_content::q2::foundation::host::{Q2Edition, Q2Motion, Q2MotionKind, Q2TraceRequest};
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{angle_vectors, donor_angle_vectors, vec3, Bounds, Plane, Vec3};
use qa_core::numeric::{Arithmetic, NumericError, NumericOps, NumericProfile};
use qa_world::body::{BodyAttachment, BodyState, LinkedBody};
use qa_world::collision::{LeafContents, Q1Move, TracePolicy};
use qa_world::movement::q1::pusher::{push_q1_pusher, Q1PushInput};
use qa_world::movement::q1::types::{Q1MovementState, Q1PhysicsEntity, Q1PusherServices, Q1Solid, Q1Trace};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q2::types::{Q2SourceTrace, Q2Trace};
use qa_world::movement::q3::types::Q3Trace;
use qa_world::movement::swept_body::{sweep_body, SweptBodyServices, SweptBodyState, SweptBodyTrace};
use qa_world::movement::types::{MovementError, TouchSurface, TraceContact, TraceHit, TraceShape};
use qa_world::registry::ActorObservation;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{arr, boolean, int, num, obj, str as save_str, SaveJson, SaveReader};
use qa_world::spatial::{
    bounds_intersect, ActorCollision, CollisionFamily, CollisionRole, CollisionShape, Q3OwnerRef, QueryRole,
    SpatialActor,
};
use qa_world::WorldError;
use thiserror::Error;

use super::new_toss::{step_q2_new_toss, NewTossEdition, NewTossOutcome, NewTossServices, NewTossWater};
use super::q2_rerelease_slide::{Q2RereleaseFlyMove, Q2SlideError, RereleaseFlyMoveServices};

/// Mirror of `TraceTarget` from donor `src/contracts/scene.ts` (canonical
/// home: `qa_world::scene::TraceTarget`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceTarget {
    /// World geometry.
    World,
    /// One brush model in caller space.
    Model {
        /// Model index.
        model: u32,
        /// Model origin.
        origin: Vec3,
        /// Model angles.
        angles: Vec3,
    },
}

/// Mirror of `TraceQuery` from donor `src/contracts/scene.ts` (canonical
/// home: `qa_world::scene::TraceQuery`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceQuery {
    /// Sweep start.
    pub start: Vec3,
    /// Sweep end.
    pub end: Vec3,
    /// Swept shape.
    pub shape: TraceShape,
    /// Geometry target.
    pub target: TraceTarget,
    /// Gameplay namespace policy.
    pub policy: TracePolicy,
    /// Numeric profile for the sweep.
    pub numeric: NumericProfile,
    /// Actor the sweep passes through.
    pub pass_actor: Option<ActorId>,
}

/// Scene trace result across families.
///
/// The per-family payloads are the real movement traces; this union only
/// restores the donor `TraceResult` (`src/contracts/scene.ts`) dispatch the
/// physics step matches on.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceResult {
    /// Quake I trace.
    Q1(Q1Trace),
    /// Quake II trace.
    Q2(Q2Trace),
    /// Quake III trace.
    Q3(Q3Trace),
}

impl TraceResult {
    /// Travel fraction consumed.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        match self {
            TraceResult::Q1(trace) => trace.fraction,
            TraceResult::Q2(trace) => trace.fraction,
            TraceResult::Q3(trace) => trace.fraction,
        }
    }

    /// Sweep end position.
    #[must_use]
    pub fn end(&self) -> Vec3 {
        match self {
            TraceResult::Q1(trace) => trace.end,
            TraceResult::Q2(trace) => trace.end,
            TraceResult::Q3(trace) => trace.end,
        }
    }

    /// Whether the sweep started inside solid.
    #[must_use]
    pub fn start_solid(&self) -> bool {
        match self {
            TraceResult::Q1(trace) => trace.start_solid,
            TraceResult::Q2(trace) => trace.start_solid,
            TraceResult::Q3(trace) => trace.start_solid,
        }
    }

    /// Whether the whole sweep stayed inside solid.
    #[must_use]
    pub fn all_solid(&self) -> bool {
        match self {
            TraceResult::Q1(trace) => trace.all_solid,
            TraceResult::Q2(trace) => trace.all_solid,
            TraceResult::Q3(trace) => trace.all_solid,
        }
    }

    /// Contact surface.
    #[must_use]
    pub fn contact(&self) -> &TraceContact {
        match self {
            TraceResult::Q1(trace) => &trace.contact,
            TraceResult::Q2(trace) => &trace.contact,
            TraceResult::Q3(trace) => &trace.contact,
        }
    }

    /// Contact plane normal, when the trace struck one.
    #[must_use]
    pub fn contact_normal(&self) -> Option<Vec3> {
        match self.contact() {
            TraceContact::None => None,
            TraceContact::Plane(plane) => Some(plane.normal),
        }
    }

    /// Hit target.
    #[must_use]
    pub fn hit(&self) -> &TraceHit {
        match self {
            TraceResult::Q1(trace) => &trace.hit,
            TraceResult::Q2(trace) => &trace.hit,
            TraceResult::Q3(trace) => &trace.hit,
        }
    }

    /// Stored source plane normal, present even when contact is none.
    #[must_use]
    pub fn source_normal(&self) -> Vec3 {
        match self {
            TraceResult::Q1(trace) => trace.source_plane.normal,
            TraceResult::Q2(trace) => trace.source_plane.normal,
            TraceResult::Q3(trace) => trace.source_plane.normal,
        }
    }

    /// Stored source plane in Hessian form.
    #[must_use]
    pub fn source_plane(&self) -> Plane {
        match self {
            TraceResult::Q1(trace) => trace.source_plane,
            TraceResult::Q2(trace) => Plane {
                normal: trace.source_plane.normal,
                distance: trace.source_plane.dist as f32,
            },
            TraceResult::Q3(trace) => Plane {
                normal: trace.source_plane.normal,
                distance: trace.source_plane.distance,
            },
        }
    }
}

impl SweptBodyTrace for TraceResult {
    fn fraction(&self) -> f64 {
        self.fraction()
    }
    fn end(&self) -> Vec3 {
        self.end()
    }
    fn all_solid(&self) -> bool {
        self.all_solid()
    }
    fn start_solid(&self) -> bool {
        self.start_solid()
    }
}

/// Mirror of `PointContentsQuery` from donor `src/contracts/scene.ts`
/// (canonical home: `qa_world::scene::PointContentsQuery`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct PointContentsQuery {
    /// Sample point.
    pub point: Vec3,
    /// Geometry target.
    pub target: TraceTarget,
    /// Gameplay namespace policy.
    pub policy: TracePolicy,
    /// Numeric profile for the sample.
    pub numeric: NumericProfile,
    /// Actor the sample passes through.
    pub pass_actor: Option<ActorId>,
}

/// Mirror of `PointContentsResult` from donor `src/contracts/scene.ts`
/// (canonical home: `qa_world::scene::PointContents`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointContents {
    /// Quake I negative contents.
    Q1 {
        /// Contents value.
        contents: i32,
    },
    /// Quake II stored and merged contents.
    Q2 {
        /// Stored leaf contents.
        stored: i32,
        /// Merged leaf contents.
        merged: i32,
    },
    /// Quake III contents.
    Q3 {
        /// Contents value.
        contents: i32,
    },
}

/// Source family selecting trace and contents namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Solidity selecting blocking and trigger behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidKind {
    /// Non-solid.
    None,
    /// Trigger volume.
    Trigger,
    /// Bounding-box solid.
    Box,
    /// Brush-model solid.
    Brush,
}

/// Collision identity of one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedSolid {
    /// Solidity.
    pub solid: SolidKind,
    /// Inline brush model, when brush-solid.
    pub model: Option<u32>,
    /// Source family.
    pub family: PhysicsFamily,
    /// Owning actor for pass-through rules.
    pub owner: Option<ActorId>,
    /// Monster flag for missile expansion rules.
    pub monster: Option<bool>,
    /// Dead-monster flag for contents mapping.
    pub dead_monster: Option<bool>,
    /// Rerelease Q1 corpse policy marker.
    pub q1_corpse: bool,
    /// Item flag widening the Q1 link padding.
    pub item: Option<bool>,
}

/// Mutable physics flags carried per actor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SharedPhysicsFlags {
    /// Team slave riding a master pusher.
    pub team_slave: Option<bool>,
    /// Touch even while non-solid.
    pub always_touch: Option<bool>,
    /// Flying locomotion.
    pub fly: Option<bool>,
    /// Swimming locomotion.
    pub swim: Option<bool>,
    /// May rest partially off ground.
    pub partial_ground: Option<bool>,
    /// Dead and untouchable, with the donor exceptions.
    pub dead: Option<bool>,
    /// Player actor (delta yaw, pusher move type).
    pub player: Option<bool>,
    /// Water level.
    pub water_level: Option<i32>,
    /// Water contents type.
    pub water_type: Option<i32>,
    /// Current enemy for flyer steering.
    pub enemy: Option<ActorId>,
    /// Accumulated pusher yaw delta.
    pub delta_yaw: Option<f64>,
}

/// Physics event kinds drained at frame boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsEventKind {
    /// Entered water.
    WaterEnter,
    /// Left water.
    WaterLeave,
    /// Landed after a fall.
    Land,
}

/// Physics event queued when no sink is installed.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsEvent {
    /// Event actor.
    pub actor: ActorId,
    /// Event kind.
    pub kind: PhysicsEventKind,
    /// Event origin.
    pub origin: Vec3,
}

/// Source-owned body physics bound to one actor.
///
/// Mirrors the donor `SourceBodyPhysics` interface (`physics.ts`); sources
/// keep their fields while collision and rider movement stay here.
pub trait SourceBodyPhysics {
    /// Current collision identity.
    fn collision(&self) -> Option<SharedSolid>;
    /// Current motion record.
    fn motion(&self) -> Option<Q2Motion>;
    /// Current physics flags.
    fn flags(&self) -> SharedPhysicsFlags;
    /// Write flag changes back to the source.
    fn write_flags(&mut self, changes: &SharedPhysicsFlags);
    /// Write angular velocity back to the source.
    fn write_angular_velocity(&mut self, velocity: Vec3);
    /// Run the source water transition.
    fn water_transition(&mut self);
}

/// Session actor registry surface used by shared physics.
///
/// Seam over donor `SessionActorRegistry` (`src/world/actors/registry.ts`);
/// only the operations the physics step calls are modeled.
pub trait PhysicsActors {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an actor to its owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Resolve a saved actor reference.
    fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
    /// Reference a saved actor in the checkpoint domain.
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Live actor observations in registry order.
    fn observations(&self) -> Vec<ActorObservation>;
    /// Assert session ownership of an actor handle.
    fn assert_owned(&self, actor: &OwnedActor);
}

/// Body table surface used by shared physics.
///
/// Seam over donor `SharedBodyTable` (`src/world/actors/body.ts`). The link
/// call takes the absolute bounds the donor table computes through the
/// physics-owned provider, so padding stays exact here.
pub trait PhysicsBodies {
    /// Read a body record.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body record.
    fn write(&mut self, actor: &OwnedActor, state: BodyState);
    /// Link a body with physics-computed absolute bounds.
    fn link(&mut self, actor: &OwnedActor, absolute_bounds: Bounds);
    /// Read the last linked snapshot.
    fn linked(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Read a body attachment.
    fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment>;
    /// Transport attachments at a committed execution boundary.
    fn transport_attachments(&mut self);
}

/// Touch contact dispatched to source callbacks.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsTouch {
    /// Touching actor.
    pub mover: OwnedActor,
    /// Touch target.
    pub other: ActorId,
    /// Contact plane, when the trace struck one.
    pub plane: Option<Plane>,
    /// Native surface detail.
    pub surface: Option<TouchSurface>,
    /// Rerelease source-trace detail, when inverted dispatch applies.
    pub source: Option<Q2SourceTrace>,
}

/// Scene query surface used by shared physics.
///
/// Seam over donor `SharedSceneQueries` (`src/world/collision/index.ts`);
/// only the operations the physics step calls are modeled.
pub trait PhysicsScene {
    /// Sweep the scene.
    fn trace(&self, query: &TraceQuery) -> TraceResult;
    /// Sweep the scene excluding extra actors.
    fn trace_excluding(&self, query: &TraceQuery, exclude: &[ActorId]) -> TraceResult;
    /// Sample contents at a point.
    fn point_contents(&self, query: &PointContentsQuery) -> PointContents;
    /// Query linked actors by bounds and role.
    fn query_actors(&self, bounds: &Bounds, role: QueryRole) -> Vec<SpatialActor>;
    /// Link a body with its collision record.
    fn link(&mut self, body: &LinkedBody, collision: &ActorCollision);
    /// Unlink an actor.
    fn unlink(&mut self, actor: &ActorId);
    /// Snapshot every spatial entry for saves.
    fn spatial_snapshot(&self) -> Vec<SpatialActor>;
    /// Clear every spatial entry before a restore.
    fn spatial_clear(&mut self);
    /// Visit Q1 trigger volumes overlapped by a linked, live mover.
    ///
    /// Seam over [`touch_q1_triggers`](qa_world::triggers::touch_q1_triggers),
    /// which needs the engine's concrete tables; the engine implements this
    /// with that function and maps contacts to [`PhysicsTouch`].
    fn touch_q1_triggers(&mut self, mover: &OwnedActor, touch: &mut dyn FnMut(PhysicsTouch));
}

/// Touch callback surface used by shared physics.
///
/// Seam over donor `ActorCallbackTable` (`src/world/actors/callbacks.ts`).
pub trait PhysicsCallbacks {
    /// Dispatch one touch contact.
    fn touch(&mut self, contact: PhysicsTouch);
}

/// Shared physics failures.
#[derive(Debug, Error)]
pub enum PhysicsError {
    /// Unsupported numeric profile.
    #[error("unsupported numeric profile: {0}")]
    Numeric(#[from] NumericError),
    /// World gravity must be finite.
    #[error("world gravity must be finite")]
    InvalidGravity,
    /// Invalid physics interval.
    #[error("invalid physics interval")]
    InvalidInterval,
    /// Nested pusher team movement is not permitted by the source frame.
    #[error("nested pusher team movement is not permitted by the source frame")]
    NestedPushTeam,
    /// Brush solidity requires an inline model.
    #[error("brush solidity requires an inline model")]
    BrushNeedsModel,
    /// Actor already has source physics.
    #[error("actor already has source physics")]
    ActorHasSource,
    /// Physics save requires a completed event boundary.
    #[error("physics save requires a completed event boundary")]
    SavePending,
    /// Q1 pusher movement failed.
    #[error("q1 pusher movement failed: {0:?}")]
    Pusher(#[from] MovementError),
    /// Rerelease slide failed.
    #[error("rerelease slide failed: {0}")]
    Slide(#[from] Q2SlideError),
}

/// Step outcome: the shared new-toss result, or `None` once settled.
pub type StepOutcome = Option<NewTossOutcome>;

/// Source slot order for candidate traversal.
pub type SourceOrder = Box<dyn Fn(&ActorId, &ActorId) -> Ordering>;
/// Blocked-pusher callback.
pub type BlockedCallback = Box<dyn FnMut(&OwnedActor, &ActorId)>;
/// Collision override.
pub type CollisionOverride = Box<dyn Fn(&OwnedActor) -> Option<SharedSolid>>;
/// Motion override.
pub type MotionOverride = Box<dyn Fn(&OwnedActor) -> Option<Q2Motion>>;
/// Flag override.
pub type FlagsOverride = Box<dyn Fn(&OwnedActor) -> SharedPhysicsFlags>;
/// Flag write-through.
pub type FlagsWriteThrough = Box<dyn FnMut(&OwnedActor, &SharedPhysicsFlags)>;
/// Angular velocity write-through.
pub type AngularVelocityWriteThrough = Box<dyn FnMut(&OwnedActor, Vec3)>;
/// Kill-velocity hook for rerelease impacts.
pub type KillVelocityHook = Box<dyn FnMut(&OwnedActor) -> bool>;
/// Q1 water transition for sourceless actors.
pub type Q1WaterTransition = Box<dyn FnMut(&OwnedActor)>;

/// Construction options for [`SharedPhysics`].
pub struct SharedPhysicsOptions {
    /// Numeric profile for the sweep math.
    pub numeric: NumericProfile,
    /// Source slot order for candidate traversal.
    pub source_order: SourceOrder,
    /// World actor, when one is linked.
    pub world_actor: Box<dyn Fn() -> Option<ActorId>>,
    /// Blocked-pusher callback.
    pub on_blocked: BlockedCallback,
    /// Optional collision override.
    pub get_collision: Option<CollisionOverride>,
    /// Optional motion override.
    pub get_motion: Option<MotionOverride>,
    /// Optional flag override.
    pub get_flags: Option<FlagsOverride>,
    /// Optional flag write-through.
    pub write_flags: Option<FlagsWriteThrough>,
    /// Optional angular velocity write-through.
    pub write_angular_velocity: Option<AngularVelocityWriteThrough>,
    /// Optional event sink; without one events queue for [`SharedPhysics::drain_events`].
    pub event: Option<Box<dyn FnMut(PhysicsEvent)>>,
    /// World gravity; defaults to 800.
    pub gravity: Option<f64>,
    /// Maximum velocity magnitude; defaults to 2000.
    pub max_velocity: Option<f64>,
    /// Q2 engine edition selecting rerelease dispatch.
    pub q2_edition: Option<Q2Edition>,
    /// Stop speed feeding new-toss friction; defaults to 100.
    pub stop_speed: Option<Box<dyn Fn() -> f64>>,
    /// Kill-velocity hook for rerelease impacts.
    pub take_kill_velocity: Option<KillVelocityHook>,
    /// Q1 water transition for sourceless actors.
    pub q1_water_transition: Option<Q1WaterTransition>,
}

#[derive(Clone)]
struct Pushed {
    actor: OwnedActor,
    origin: Vec3,
    angles: Vec3,
    delta_yaw: f64,
}

const DEFAULT_MASK: i32 = 0x0202_0003;

fn zero() -> Vec3 {
    vec3(0.0, 0.0, 0.0)
}

fn motion_kind_name(kind: &Q2MotionKind) -> &'static str {
    match kind {
        Q2MotionKind::Stationary => "stationary",
        Q2MotionKind::Push => "push",
        Q2MotionKind::Stop => "stop",
        Q2MotionKind::Toss => "toss",
        Q2MotionKind::NewToss => "new-toss",
        Q2MotionKind::Bounce => "bounce",
        Q2MotionKind::WallBounce => "wall-bounce",
        Q2MotionKind::Fly => "fly",
        Q2MotionKind::FlyMissile => "fly-missile",
        Q2MotionKind::Step => "step",
    }
}

fn motion_kind_named(name: &str) -> Option<Q2MotionKind> {
    match name {
        "stationary" => Some(Q2MotionKind::Stationary),
        "push" => Some(Q2MotionKind::Push),
        "stop" => Some(Q2MotionKind::Stop),
        "toss" => Some(Q2MotionKind::Toss),
        "new-toss" => Some(Q2MotionKind::NewToss),
        "bounce" => Some(Q2MotionKind::Bounce),
        "wall-bounce" => Some(Q2MotionKind::WallBounce),
        "fly" => Some(Q2MotionKind::Fly),
        "fly-missile" => Some(Q2MotionKind::FlyMissile),
        "step" => Some(Q2MotionKind::Step),
        _ => None,
    }
}

/// The scheduler calls one actor at a time. This owner never starts a family loop.
pub struct SharedPhysics {
    options: SharedPhysicsOptions,
    actors: Box<dyn PhysicsActors>,
    bodies: Box<dyn PhysicsBodies>,
    scene: Box<dyn PhysicsScene>,
    callbacks: Box<dyn PhysicsCallbacks>,
    numeric: NumericOps,
    solids: HashMap<ActorId, SharedSolid>,
    collisions: HashMap<ActorId, ActorCollision>,
    motions: HashMap<ActorId, Q2Motion>,
    flags: HashMap<ActorId, SharedPhysicsFlags>,
    events: Vec<PhysicsEvent>,
    sources: HashMap<ActorId, Box<dyn SourceBodyPhysics>>,
    push_transaction: Option<Vec<Pushed>>,
    world_gravity: f64,
    rerelease_movement: Q2RereleaseMovementContext,
    rerelease_fly_move: Q2RereleaseFlyMove,
}

impl SharedPhysics {
    /// Create shared physics over engine tables.
    pub fn new(
        options: SharedPhysicsOptions,
        actors: Box<dyn PhysicsActors>,
        bodies: Box<dyn PhysicsBodies>,
        scene: Box<dyn PhysicsScene>,
        callbacks: Box<dyn PhysicsCallbacks>,
    ) -> Result<Self, PhysicsError> {
        let numeric = NumericOps::select(options.numeric)?;
        let world_gravity = options.gravity.unwrap_or(800.0);
        Ok(Self {
            options,
            actors,
            bodies,
            scene,
            callbacks,
            numeric,
            solids: HashMap::new(),
            collisions: HashMap::new(),
            motions: HashMap::new(),
            flags: HashMap::new(),
            events: Vec::new(),
            sources: HashMap::new(),
            push_transaction: None,
            world_gravity,
            rerelease_movement: Q2RereleaseMovementContext::new(),
            rerelease_fly_move: Q2RereleaseFlyMove::new(numeric),
        })
    }

    fn vector(&self, x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 {
            x: self.numeric.store(x),
            y: self.numeric.store(y),
            z: self.numeric.store(z),
        }
    }

    fn add(&self, a: Vec3, b: Vec3) -> Vec3 {
        self.vector(
            self.numeric.add(f64::from(a.x), f64::from(b.x)),
            self.numeric.add(f64::from(a.y), f64::from(b.y)),
            self.numeric.add(f64::from(a.z), f64::from(b.z)),
        )
    }

    fn sub(&self, a: Vec3, b: Vec3) -> Vec3 {
        self.vector(
            self.numeric.sub(f64::from(a.x), f64::from(b.x)),
            self.numeric.sub(f64::from(a.y), f64::from(b.y)),
            self.numeric.sub(f64::from(a.z), f64::from(b.z)),
        )
    }

    fn scale(&self, v: Vec3, value: f64) -> Vec3 {
        self.vector(
            self.numeric.mul(f64::from(v.x), value),
            self.numeric.mul(f64::from(v.y), value),
            self.numeric.mul(f64::from(v.z), value),
        )
    }

    fn dot(&self, a: Vec3, b: Vec3) -> f64 {
        self.numeric.add(
            self.numeric.add(
                self.numeric.mul(f64::from(a.x), f64::from(b.x)),
                self.numeric.mul(f64::from(a.y), f64::from(b.y)),
            ),
            self.numeric.mul(f64::from(a.z), f64::from(b.z)),
        )
    }

    fn moving(value: Vec3) -> bool {
        value.x != 0.0 || value.y != 0.0 || value.z != 0.0
    }

    fn live(&self, actor: &OwnedActor) -> bool {
        self.actors.is_live(actor.id())
    }

    fn solid(&self, actor: &OwnedActor) -> Option<SharedSolid> {
        if let Some(source) = self.sources.get(actor.id()) {
            if let Some(solid) = source.collision() {
                return Some(solid);
            }
        }
        if let Some(get) = self.options.get_collision.as_ref() {
            if let Some(solid) = get(actor) {
                return Some(solid);
            }
        }
        self.solids.get(actor.id()).cloned()
    }

    fn motion(&self, actor: &OwnedActor) -> Option<Q2Motion> {
        if let Some(source) = self.sources.get(actor.id()) {
            if let Some(motion) = source.motion() {
                return Some(motion);
            }
        }
        if let Some(get) = self.options.get_motion.as_ref() {
            if let Some(motion) = get(actor) {
                return Some(motion);
            }
        }
        self.motions.get(actor.id()).cloned()
    }

    /// Bind source-owned physics to an actor.
    pub fn bind_source(&mut self, actor: &OwnedActor, source: Box<dyn SourceBodyPhysics>) -> Result<(), PhysicsError> {
        self.actors.assert_owned(actor);
        if self.sources.contains_key(actor.id()) {
            return Err(PhysicsError::ActorHasSource);
        }
        self.sources.insert(actor.id().clone(), source);
        Ok(())
    }

    /// Release a bound source without releasing the actor.
    pub fn unbind_source(&mut self, actor: &ActorId) {
        self.sources.remove(actor);
    }

    /// Drop retained physics state after the engine releases an actor.
    ///
    /// The donor subscribes to registry releases; the engine calls this
    /// instead so the borrow stays on the host side.
    pub fn release_actor(&mut self, actor: &ActorId) {
        self.sources.remove(actor);
        self.solids.remove(actor);
        self.collisions.remove(actor);
        self.motions.remove(actor);
        self.flags.remove(actor);
    }

    fn family(&self, actor: &OwnedActor) -> PhysicsFamily {
        self.solid(actor).map(|solid| solid.family).unwrap_or(PhysicsFamily::Q2)
    }

    /// Current world gravity.
    #[must_use]
    pub fn gravity(&self) -> f64 {
        self.world_gravity
    }

    /// Set world gravity.
    pub fn set_world_gravity(&mut self, value: f64) -> Result<(), PhysicsError> {
        if !value.is_finite() {
            return Err(PhysicsError::InvalidGravity);
        }
        self.world_gravity = value;
        Ok(())
    }

    /// Capture the save checkpoint as donor-shaped JSON.
    pub fn capture(&self) -> Result<SaveJson, PhysicsError> {
        if self.push_transaction.is_some() || !self.events.is_empty() {
            return Err(PhysicsError::SavePending);
        }
        let mut collisions: Vec<(&ActorId, &ActorCollision)> = self.collisions.iter().collect();
        collisions.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        let mut spatial = self.scene.spatial_snapshot();
        spatial.sort_by_key(|entry| (entry.body.actor.slot(), entry.body.actor.generation()));
        let mut solids: Vec<(&ActorId, &SharedSolid)> = self.solids.iter().collect();
        solids.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        let mut motions: Vec<(&ActorId, &Q2Motion)> = self.motions.iter().collect();
        motions.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        let mut flags: Vec<(&ActorId, &SharedPhysicsFlags)> = self.flags.iter().collect();
        flags.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        Ok(obj(vec![
            ("gravity", num(self.world_gravity)),
            ("rereleaseMovement", write_vector(self.rerelease_movement.capture())),
            (
                "collisions",
                arr(collisions
                    .iter()
                    .map(|(actor, collision)| {
                        obj(vec![
                            ("actor", write_saved_actor(SavedActorId::from(*actor))),
                            ("collision", write_collision(collision)),
                        ])
                    })
                    .collect()),
            ),
            (
                "spatial",
                arr(spatial
                    .iter()
                    .map(|entry| {
                        obj(vec![
                            ("actor", write_saved_actor(SavedActorId::from(&entry.body.actor))),
                            ("collision", write_collision(&entry.collision)),
                        ])
                    })
                    .collect()),
            ),
            (
                "solids",
                arr(solids.iter().map(|(actor, solid)| write_solid(actor, solid)).collect()),
            ),
            (
                "motions",
                arr(motions.iter().map(|(_, motion)| write_motion(motion)).collect()),
            ),
            (
                "flags",
                arr(flags.iter().map(|(actor, flags)| write_flags(actor, flags)).collect()),
            ),
        ]))
    }

    /// Restore a checkpoint captured by [`SharedPhysics::capture`].
    ///
    /// Entries owned by `reconstructed` actors are retained across the clear
    /// so a partial restore never drops live source state.
    pub fn restore_checkpoint(
        &mut self,
        reader: &SaveReader,
        require_exact_collisions: bool,
        reconstructed: &dyn Fn(&ActorId) -> bool,
    ) -> Result<(), WorldError> {
        let retained_solids: Vec<(ActorId, SharedSolid)> = self
            .solids
            .iter()
            .filter(|(actor, _)| reconstructed(actor))
            .map(|(actor, solid)| (actor.clone(), solid.clone()))
            .collect();
        let retained_motions: Vec<(ActorId, Q2Motion)> = self
            .motions
            .iter()
            .filter(|(actor, _)| reconstructed(actor))
            .map(|(actor, motion)| (actor.clone(), motion.clone()))
            .collect();
        let retained_flags: Vec<(ActorId, SharedPhysicsFlags)> = self
            .flags
            .iter()
            .filter(|(actor, _)| reconstructed(actor))
            .map(|(actor, flags)| (actor.clone(), flags.clone()))
            .collect();
        let retained_collisions: Vec<(ActorId, ActorCollision)> = self
            .collisions
            .iter()
            .filter(|(actor, _)| reconstructed(actor))
            .map(|(actor, collision)| (actor.clone(), collision.clone()))
            .collect();
        self.world_gravity = reader.field("gravity").finite()?;
        self.rerelease_movement
            .restore(read_vector(reader.field("rereleaseMovement"))?);
        self.solids.clear();
        self.motions.clear();
        self.flags.clear();
        self.collisions.clear();
        let collisions = reader.field("collisions");
        if require_exact_collisions || !collisions.is_missing() {
            let entries: Vec<(OwnedActor, ActorCollision)> = collisions.list(|value| {
                let actor = self.restore_owner(value.field("actor"))?;
                let collision = self.read_collision(&value.field("collision"))?;
                Ok((actor, collision))
            })?;
            for (actor, collision) in entries {
                if self.collisions.contains_key(actor.id()) {
                    return Err(reader.fail("Duplicate saved collision actor"));
                }
                self.collisions.insert(actor.id().clone(), collision);
            }
        }
        let solids: Vec<(OwnedActor, SharedSolid)> = reader.field("solids").list(|value| {
            let actor = self.restore_owner(value.field("actor"))?;
            let solid = value.field("solid").choice_str(&["none", "trigger", "box", "brush"])?;
            let family = value.field("family").choice_str(&["q1", "q2", "q3"])?;
            Ok((
                actor,
                SharedSolid {
                    solid: match solid.as_str() {
                        "none" => SolidKind::None,
                        "trigger" => SolidKind::Trigger,
                        "box" => SolidKind::Box,
                        _ => SolidKind::Brush,
                    },
                    model: value
                        .field("model")
                        .nullable(|model| model.integer(0))?
                        .map(|model| model as u32),
                    family: match family.as_str() {
                        "q1" => PhysicsFamily::Q1,
                        "q2" => PhysicsFamily::Q2,
                        _ => PhysicsFamily::Q3,
                    },
                    owner: value.field("owner").nullable(|owner| self.restore_reference(owner))?,
                    monster: optional_bool(&value, "monster")?,
                    dead_monster: optional_bool(&value, "deadMonster")?,
                    q1_corpse: read_corpse(&value)?,
                    item: optional_bool(&value, "item")?,
                },
            ))
        })?;
        for (actor, solid) in solids {
            self.solids.insert(actor.id().clone(), solid);
        }
        let motions: Vec<(OwnedActor, Q2Motion)> = reader.field("motions").list(|value| {
            let actor = self.restore_owner(value.field("actor"))?;
            let kind = value.field("kind").choice_str(&[
                "stationary",
                "push",
                "stop",
                "toss",
                "new-toss",
                "bounce",
                "wall-bounce",
                "fly",
                "fly-missile",
                "step",
            ])?;
            let Some(kind) = motion_kind_named(&kind) else {
                return Err(value.fail("Unknown saved motion kind"));
            };
            Ok((
                actor.clone(),
                Q2Motion {
                    actor,
                    kind,
                    velocity: read_vector(value.field("velocity"))?,
                    angular_velocity: read_vector(value.field("angularVelocity"))?,
                    gravity: value.field("gravity").number()?,
                    gravity_vector: read_vector(value.field("gravityVector"))?,
                    clip_mask: value.field("clipMask").number()? as i32,
                    owner: value.field("owner").nullable(|owner| self.restore_reference(owner))?,
                },
            ))
        })?;
        for (actor, motion) in motions {
            self.motions.insert(actor.id().clone(), motion);
        }
        let flags: Vec<(OwnedActor, SharedPhysicsFlags)> = reader.field("flags").list(|value| {
            let actor = self.restore_owner(value.field("actor"))?;
            Ok((
                actor,
                SharedPhysicsFlags {
                    team_slave: optional_bool(&value, "teamSlave")?,
                    always_touch: optional_bool(&value, "alwaysTouch")?,
                    fly: optional_bool(&value, "fly")?,
                    swim: optional_bool(&value, "swim")?,
                    partial_ground: optional_bool(&value, "partialGround")?,
                    dead: optional_bool(&value, "dead")?,
                    player: optional_bool(&value, "player")?,
                    water_level: optional_number(&value, "waterLevel")?.map(|level| level as i32),
                    water_type: optional_number(&value, "waterType")?.map(|kind| kind as i32),
                    delta_yaw: optional_number(&value, "deltaYaw")?,
                    enemy: if value.field("enemy").is_missing() {
                        None
                    } else {
                        value.field("enemy").nullable(|enemy| self.restore_reference(enemy))?
                    },
                },
            ))
        })?;
        for (actor, flag) in flags {
            self.flags.insert(actor.id().clone(), flag);
        }
        for (actor, solid) in retained_solids {
            self.solids.remove(&actor);
            self.solids.insert(actor, solid);
        }
        for (actor, motion) in retained_motions {
            self.motions.remove(&actor);
            self.motions.insert(actor, motion);
        }
        for (actor, flags) in retained_flags {
            self.flags.remove(&actor);
            self.flags.insert(actor, flags);
        }
        for (actor, collision) in retained_collisions {
            self.collisions.remove(&actor);
            self.collisions.insert(actor, collision);
        }
        Ok(())
    }

    /// Restore spatial links captured by [`SharedPhysics::capture`].
    pub fn restore_spatial(
        &mut self,
        reader: &SaveReader,
        reconstructed: &dyn Fn(&ActorId) -> bool,
    ) -> Result<(), WorldError> {
        let retained: Vec<SpatialActor> = self
            .scene
            .spatial_snapshot()
            .into_iter()
            .filter(|entry| reconstructed(&entry.body.actor))
            .collect();
        self.scene.spatial_clear();
        let entries: Vec<(LinkedBody, ActorCollision)> = reader
            .field("spatial")
            .list(|value| {
                let saved = read_saved_actor(value.field("actor"))?;
                let Some(actor) = self.actors.resolve_saved(saved) else {
                    return Err(value.fail("Saved spatial actor has no restored identity"));
                };
                let collision = self.read_collision(&value.field("collision"))?;
                if reconstructed(actor.id()) {
                    return Ok(None);
                }
                let Some(body) = self.bodies.linked(actor.id()) else {
                    return Err(value.fail("Saved spatial actor has no retained body link"));
                };
                Ok(Some((body, collision)))
            })?
            .into_iter()
            .flatten()
            .collect();
        for (body, collision) in entries {
            self.scene.link(&body, &collision);
        }
        for entry in retained {
            self.scene.link(&entry.body, &entry.collision);
        }
        Ok(())
    }

    fn restore_owner(&self, reader: SaveReader) -> Result<OwnedActor, WorldError> {
        let Some(actor) = self.actors.resolve_saved(read_saved_actor(reader.clone())?) else {
            return Err(reader.fail("Missing shared physics actor"));
        };
        Ok(actor)
    }

    fn restore_reference(&self, reader: SaveReader) -> Result<ActorId, WorldError> {
        Ok(self.actors.reference_saved(read_saved_actor(reader)?))
    }

    fn read_collision(&self, collision: &SaveReader) -> Result<ActorCollision, WorldError> {
        let shape = collision.field("shape");
        let kind = shape.field("kind").choice_str(&["box", "capsule", "model"])?;
        let family = collision.field("family").choice_str(&["q1", "q2", "q3"])?;
        let role = collision.field("role").choice_str(&["solid", "trigger"])?;
        let q3_owner = collision.field("q3Owner");
        Ok(ActorCollision {
            family: match family.as_str() {
                "q1" => CollisionFamily::Q1,
                "q2" => CollisionFamily::Q2,
                _ => CollisionFamily::Q3,
            },
            shape: match kind.as_str() {
                "model" => CollisionShape::Model(shape.field("model").integer(0)? as u32),
                "capsule" => CollisionShape::Capsule,
                _ => CollisionShape::Box,
            },
            contents: collision.field("contents").number()? as i32,
            owner: collision
                .field("owner")
                .nullable(|owner| self.restore_reference(owner))?,
            role: if role == "trigger" {
                CollisionRole::Trigger
            } else {
                CollisionRole::Solid
            },
            monster: collision.field("monster").boolean()?,
            dead_monster: collision.field("deadMonster").boolean()?,
            q1_corpse: read_corpse(collision)?,
            q3_owner: if q3_owner.is_missing() {
                None
            } else {
                q3_owner.nullable(|owner| {
                    Ok(Q3OwnerRef {
                        entity_number: owner.field("entityNumber").integer(0)? as i32,
                        owner_number: owner.field("ownerNumber").integer(0)? as i32,
                    })
                })?
            },
        })
    }

    /// Collision identity of a live actor.
    pub fn solid_of(&self, actor: &ActorId) -> Option<SharedSolid> {
        let owned = self.actors.resolve_owned(actor)?;
        self.solid(&owned)
    }

    /// Motion record of a live actor.
    pub fn motion_of(&self, actor: &ActorId) -> Option<Q2Motion> {
        let owned = self.actors.resolve_owned(actor)?;
        self.motion(&owned)
    }

    /// Whether a live actor is brush-solid.
    pub fn is_brush(&self, actor: &ActorId) -> bool {
        self.actors
            .resolve_owned(actor)
            .is_some_and(|owned| self.solid(&owned).is_some_and(|solid| solid.solid == SolidKind::Brush))
    }

    /// Live collision record for an actor (donor `scene.spatial.get`).
    ///
    /// C4 sibling edit: the simulation combat range reads linked collision
    /// roles/shapes for weapon targeting.
    #[must_use]
    pub fn collision_of(&self, actor: &ActorId) -> Option<ActorCollision> {
        self.collisions.get(actor).cloned()
    }

    /// Rerelease movement context for Q2-rerelease player movement.
    ///
    /// C4 sibling edit: the simulation movement range drives rerelease
    /// physics through this context.
    #[must_use]
    pub fn rerelease_movement(&self) -> &Q2RereleaseMovementContext {
        &self.rerelease_movement
    }

    /// Write back a rerelease movement context after a step.
    ///
    /// C5 sibling edit: movement sessions clone the shared context per step
    /// (the physics table cannot stay borrowed across movement callbacks)
    /// and store the stepped context here.
    pub fn set_rerelease_movement(&mut self, context: Q2RereleaseMovementContext) {
        self.rerelease_movement = context;
    }

    /// Sample point contents through the scene (donor `scene.pointContents`).
    ///
    /// C4 sibling edit: the simulation queries world contents for hazard
    /// checks; the scene backend stays with the collision lane.
    #[must_use]
    pub fn sample_contents(&self, query: &PointContentsQuery) -> PointContents {
        self.scene.point_contents(query)
    }

    /// Drain queued physics events.
    pub fn drain_events(&mut self) -> Vec<PhysicsEvent> {
        std::mem::take(&mut self.events)
    }

    fn emit(&mut self, event: PhysicsEvent) {
        if let Some(sink) = self.options.event.as_mut() {
            sink(event);
        } else {
            self.events.push(event);
        }
    }

    fn actor_flags(&self, actor: &OwnedActor) -> SharedPhysicsFlags {
        let mut flags = self.flags.get(actor.id()).cloned().unwrap_or_default();
        let extra = self
            .sources
            .get(actor.id())
            .map(|source| source.flags())
            .or_else(|| self.options.get_flags.as_ref().map(|get| get(actor)));
        if let Some(extra) = extra {
            merge_flags(&mut flags, &extra);
        }
        flags
    }

    /// Merge flag changes into retained state and write them through.
    pub fn set_flags(&mut self, actor: &OwnedActor, changes: SharedPhysicsFlags) {
        if !self.live(actor) {
            return;
        }
        let mut merged = self.actor_flags(actor);
        merge_flags(&mut merged, &changes);
        self.flags.insert(actor.id().clone(), merged);
        if let Some(source) = self.sources.get_mut(actor.id()) {
            source.write_flags(&changes);
        } else if let Some(write) = self.options.write_flags.as_mut() {
            write(actor, &changes);
        }
    }

    fn write_angular_velocity(&mut self, actor: &OwnedActor, value: Vec3) {
        if let Some(source) = self.sources.get_mut(actor.id()) {
            source.write_angular_velocity(value);
        } else if let Some(write) = self.options.write_angular_velocity.as_mut() {
            write(actor, value);
        }
    }

    /// Set raw solidity for an actor.
    pub fn set_solid(
        &mut self,
        actor: &OwnedActor,
        solid: SolidKind,
        model: Option<u32>,
        family: PhysicsFamily,
        owner: Option<ActorId>,
    ) -> Result<(), PhysicsError> {
        self.actors.assert_owned(actor);
        if solid == SolidKind::Brush && model.is_none() {
            return Err(PhysicsError::BrushNeedsModel);
        }
        let id = actor.id().clone();
        self.solids.insert(
            id.clone(),
            SharedSolid {
                solid,
                model,
                family,
                owner,
                monster: None,
                dead_monster: None,
                q1_corpse: false,
                item: None,
            },
        );
        if let Some(linked) = self.bodies.linked(&id) {
            self.linked(&linked);
        }
        Ok(())
    }

    /// Set an exact collision record for an actor.
    pub fn set_collision(&mut self, actor: &OwnedActor, collision: ActorCollision) {
        self.actors.assert_owned(actor);
        let id = actor.id().clone();
        self.collisions.insert(id.clone(), collision.clone());
        self.solids.insert(
            id.clone(),
            SharedSolid {
                solid: if collision.role == CollisionRole::Trigger {
                    SolidKind::Trigger
                } else if matches!(collision.shape, CollisionShape::Model(_)) {
                    SolidKind::Brush
                } else {
                    SolidKind::Box
                },
                model: match collision.shape {
                    CollisionShape::Model(model) => Some(model),
                    _ => None,
                },
                family: match collision.family {
                    CollisionFamily::Q1 => PhysicsFamily::Q1,
                    CollisionFamily::Q2 => PhysicsFamily::Q2,
                    CollisionFamily::Q3 => PhysicsFamily::Q3,
                },
                owner: collision.owner.clone(),
                monster: Some(collision.monster),
                dead_monster: Some(collision.dead_monster),
                q1_corpse: collision.q1_corpse,
                item: None,
            },
        );
        if let Some(linked) = self.bodies.linked(&id) {
            self.linked(&linked);
        }
    }

    /// Set a motion record for an actor.
    pub fn set_motion(&mut self, motion: Q2Motion) {
        self.actors.assert_owned(&motion.actor);
        let id = motion.actor.id().clone();
        if let Some(body) = self.bodies.read(&id) {
            let mut updated = body;
            updated.velocity = motion.velocity;
            self.bodies.write(&motion.actor, updated);
        }
        self.motions.insert(id.clone(), motion);
        if let Some(linked) = self.bodies.linked(&id) {
            self.linked(&linked);
        }
    }

    fn absolute_bounds(&self, actor: &OwnedActor, state: &BodyState) -> Bounds {
        let solid = self.solid(actor);
        let mut bounds = Bounds {
            min: self.add(state.origin, state.bounds.min),
            max: self.add(state.origin, state.bounds.max),
        };
        if solid.as_ref().is_some_and(|solid| solid.solid == SolidKind::Brush) && Self::moving(state.angles) {
            let extent = Vec3 {
                x: f64::from(state.bounds.min.x)
                    .abs()
                    .max(f64::from(state.bounds.max.x).abs()) as f32,
                y: f64::from(state.bounds.min.y)
                    .abs()
                    .max(f64::from(state.bounds.max.y).abs()) as f32,
                z: f64::from(state.bounds.min.z)
                    .abs()
                    .max(f64::from(state.bounds.max.z).abs()) as f32,
            };
            let radius = self.dot(extent, extent).sqrt();
            let offset = Vec3 {
                x: radius as f32,
                y: radius as f32,
                z: radius as f32,
            };
            bounds = Bounds {
                min: self.sub(state.origin, offset),
                max: self.add(state.origin, offset),
            };
        }
        let pad = if solid
            .as_ref()
            .is_some_and(|solid| solid.family == PhysicsFamily::Q1 && solid.item == Some(true))
        {
            vec3(15.0, 15.0, 0.0)
        } else {
            vec3(1.0, 1.0, 1.0)
        };
        Bounds {
            min: self.sub(bounds.min, pad),
            max: self.add(bounds.max, pad),
        }
    }

    fn linked(&mut self, body: &LinkedBody) {
        let Some(actor) = self.actors.resolve_owned(&body.actor) else {
            return;
        };
        if let Some(exact) = self.collisions.get(actor.id()).cloned() {
            self.scene.link(body, &exact);
            return;
        }
        let solid = self.solid(&actor);
        let world = (self.options.world_actor)();
        if solid.is_none()
            || solid.as_ref().is_some_and(|solid| solid.solid == SolidKind::None)
            || world.as_ref().is_some_and(|world| world == actor.id())
        {
            self.scene.unlink(actor.id());
            return;
        }
        let solid = solid.expect("solid checked");
        let shape = if solid.solid == SolidKind::Brush && solid.model.is_some() {
            CollisionShape::Model(solid.model.unwrap_or(0))
        } else {
            CollisionShape::Box
        };
        let contents = if solid.family == PhysicsFamily::Q1 {
            -2
        } else if solid.solid == SolidKind::Brush {
            1
        } else if solid.dead_monster == Some(true) {
            0x400_0000
        } else {
            0x200_0000
        };
        let motion = self.motion(&actor);
        self.scene.link(
            body,
            &ActorCollision {
                family: match solid.family {
                    PhysicsFamily::Q1 => CollisionFamily::Q1,
                    PhysicsFamily::Q2 => CollisionFamily::Q2,
                    PhysicsFamily::Q3 => CollisionFamily::Q3,
                },
                shape,
                contents,
                owner: motion.map(|motion| motion.owner).unwrap_or(solid.owner),
                q1_corpse: solid.q1_corpse,
                role: if solid.solid == SolidKind::Trigger {
                    CollisionRole::Trigger
                } else {
                    CollisionRole::Solid
                },
                monster: solid.monster.unwrap_or(false),
                dead_monster: solid.dead_monster.unwrap_or(false),
                q3_owner: None,
            },
        );
    }

    /// Link a body with physics-computed absolute bounds and scene publication.
    fn link_body(&mut self, actor: &OwnedActor) {
        let Some(state) = self.bodies.read(actor.id()) else {
            return;
        };
        let bounds = self.absolute_bounds(actor, &state);
        self.bodies.link(actor, bounds);
        if let Some(linked) = self.bodies.linked(actor.id()) {
            self.linked(&linked);
        }
    }

    fn policy(&self, family: PhysicsFamily, mask: i32, movement: Q1Move) -> TracePolicy {
        match family {
            PhysicsFamily::Q1 => TracePolicy::Q1 { movement },
            PhysicsFamily::Q2 => TracePolicy::Q2 {
                contents_mask: mask,
                leaf_contents: LeafContents::Merged,
            },
            PhysicsFamily::Q3 => TracePolicy::Q3 { contents_mask: mask },
        }
    }

    /// Sweep the scene for a Q2 trace request under a family policy.
    pub fn trace(&self, request: &Q2TraceRequest, family: PhysicsFamily) -> TraceResult {
        let query = TraceQuery {
            start: request.start,
            end: request.end,
            shape: match request.bounds {
                None => TraceShape::Point,
                Some(bounds) => TraceShape::Box(bounds),
            },
            target: TraceTarget::World,
            policy: self.policy(family, request.mask, Q1Move::Normal),
            numeric: self.options.numeric,
            pass_actor: request.ignore.clone(),
        };
        if request.exclude.is_empty() {
            self.scene.trace(&query)
        } else {
            self.scene.trace_excluding(&query, &request.exclude)
        }
    }

    fn body_trace(
        &self,
        actor: &OwnedActor,
        start: Vec3,
        end: Vec3,
        exclude: &[ActorId],
        exact_mask: bool,
        bounds: Option<Bounds>,
    ) -> TraceResult {
        let Some(body) = self.bodies.read(actor.id()) else {
            panic!("cannot trace an actor without a body");
        };
        let family = self.family(actor);
        let motion = self.motion(actor);
        let solid = self.solid(actor).map(|solid| solid.solid);
        let movement = if family == PhysicsFamily::Q1
            && motion
                .as_ref()
                .is_some_and(|motion| motion.kind == Q2MotionKind::FlyMissile)
        {
            Q1Move::Missile
        } else if family == PhysicsFamily::Q1 && matches!(solid, Some(SolidKind::None) | Some(SolidKind::Trigger)) {
            Q1Move::NoMonsters
        } else {
            Q1Move::Normal
        };
        let mask = if exact_mask {
            motion.as_ref().map(|motion| motion.clip_mask).unwrap_or(0)
        } else {
            let mask = motion.as_ref().map(|motion| motion.clip_mask).unwrap_or(0);
            if mask == 0 {
                3
            } else {
                mask
            }
        };
        let query = TraceQuery {
            start,
            end,
            shape: TraceShape::Box(bounds.unwrap_or(body.bounds)),
            target: TraceTarget::World,
            policy: self.policy(family, mask, movement),
            numeric: self.options.numeric,
            pass_actor: Some(actor.id().clone()),
        };
        if exclude.is_empty() {
            self.scene.trace(&query)
        } else {
            self.scene.trace_excluding(&query, exclude)
        }
    }

    fn hit_actor(&self, trace: &TraceResult) -> Option<ActorId> {
        match trace.hit() {
            TraceHit::Actor { actor } => Some(actor.clone()),
            TraceHit::World { .. } => (self.options.world_actor)(),
            TraceHit::None => None,
        }
    }

    /// Touch triggers overlapped by a linked, live actor.
    pub fn touch_triggers(&mut self, actor: &OwnedActor) {
        let id = actor.id().clone();
        if self.bodies.linked(&id).is_none() || !self.live(actor) {
            return;
        }
        let flags = self.actor_flags(actor);
        let family = self.family(actor);
        if family == PhysicsFamily::Q1 {
            let actor = actor.clone();
            let callbacks = &mut *self.callbacks;
            let scene = &mut self.scene;
            scene.touch_q1_triggers(&actor, &mut |contact| callbacks.touch(contact));
            return;
        }
        if flags.dead == Some(true)
            && (flags.player == Some(true) || self.solid(actor).is_some_and(|solid| solid.monster == Some(true)))
        {
            return;
        }
        let Some(linked) = self.bodies.linked(&id) else {
            return;
        };
        let candidates = self.scene.query_actors(&linked.absolute_bounds, QueryRole::Trigger);
        for candidate in candidates {
            if !self.live(actor) {
                break;
            }
            let trigger_id = candidate.body.actor.clone();
            if trigger_id == id {
                continue;
            }
            let Some(trigger) = self.actors.resolve_owned(&trigger_id) else {
                continue;
            };
            let (Some(current), Some(moving)) = (self.bodies.linked(&trigger_id), self.bodies.linked(&id)) else {
                continue;
            };
            if self
                .solid(&trigger)
                .is_none_or(|solid| solid.solid != SolidKind::Trigger)
            {
                continue;
            }
            if !bounds_intersect(&current.absolute_bounds, &moving.absolute_bounds) {
                continue;
            }
            self.callbacks.touch(PhysicsTouch {
                mover: trigger,
                other: id.clone(),
                plane: None,
                surface: None,
                source: None,
            });
        }
    }

    fn impact(&mut self, actor: &OwnedActor, trace: &TraceResult) {
        let Some(other_id) = self.hit_actor(trace) else {
            return;
        };
        if !self.live(actor) {
            return;
        }
        let other = self.actors.resolve_owned(&other_id);
        let plane = match trace.contact() {
            TraceContact::Plane(plane) => *plane,
            TraceContact::None => trace.source_plane(),
        };
        let surface = match trace {
            TraceResult::Q2(q2) => q2.surface.as_ref().map(|surface| TouchSurface {
                name: surface.name.clone(),
                native_flags: surface.flags,
                native_value: surface.value,
            }),
            TraceResult::Q1(q1) => q1.surface_flags.map(|flags| TouchSurface {
                name: String::new(),
                native_flags: flags & 0x86,
                native_value: 0,
            }),
            TraceResult::Q3(_) => None,
        };
        if self.family(actor) == PhysicsFamily::Q2
            && self.options.q2_edition == Some(Q2Edition::Rerelease)
            && matches!(trace, TraceResult::Q2(_))
        {
            let TraceResult::Q2(q2) = trace else {
                return;
            };
            if self.solid(actor).is_none_or(|solid| solid.solid != SolidKind::None)
                || self.actor_flags(actor).always_touch == Some(true)
            {
                self.callbacks.touch(PhysicsTouch {
                    mover: actor.clone(),
                    other: other_id.clone(),
                    plane: Some(plane),
                    surface: surface.clone(),
                    source: Some(Q2SourceTrace {
                        trace: q2.clone(),
                        inverted: false,
                    }),
                });
            }
            if let Some(other) = other {
                if self.live(&other)
                    && (self.solid(&other).is_none_or(|solid| solid.solid != SolidKind::None)
                        || self.actor_flags(&other).always_touch == Some(true))
                {
                    self.callbacks.touch(PhysicsTouch {
                        mover: other,
                        other: actor.id().clone(),
                        plane: Some(plane),
                        surface,
                        source: Some(Q2SourceTrace {
                            trace: q2.clone(),
                            inverted: true,
                        }),
                    });
                }
            }
            return;
        }
        if self.solid(actor).is_none_or(|solid| solid.solid != SolidKind::None) {
            self.callbacks.touch(PhysicsTouch {
                mover: actor.clone(),
                other: other_id.clone(),
                plane: Some(plane),
                surface,
                source: None,
            });
        }
        if let Some(other) = other {
            if self.live(actor)
                && self.live(&other)
                && self.solid(&other).is_none_or(|solid| solid.solid != SolidKind::None)
            {
                self.callbacks.touch(PhysicsTouch {
                    mover: other,
                    other: actor.id().clone(),
                    plane: None,
                    surface: None,
                    source: None,
                });
            }
        }
    }

    fn push_entity(&mut self, actor: &OwnedActor, displacement: Vec3, exclude: &[ActorId]) -> TraceResult {
        let Some(initial) = self.bodies.read(actor.id()) else {
            panic!("cannot push an actor without a body");
        };
        let end = self.add(initial.origin, displacement);
        loop {
            let trace = self.body_trace(actor, initial.origin, end, exclude, false, None);
            let Some(current) = self.bodies.read(actor.id()) else {
                return trace;
            };
            let mut moved = current;
            moved.origin = trace.end();
            self.bodies.write(actor, moved);
            self.link_body(actor);
            if trace.fraction() != 1.0 {
                let hit = self.hit_actor(&trace);
                self.impact(actor, &trace);
                if self.family(actor) != PhysicsFamily::Q1
                    && hit.as_ref().is_some_and(|hit| !self.actors.is_live(hit))
                    && self.live(actor)
                {
                    if let Some(changed) = self.bodies.read(actor.id()) {
                        let mut restored = changed;
                        restored.origin = initial.origin;
                        self.bodies.write(actor, restored);
                        self.link_body(actor);
                        continue;
                    }
                }
            }
            if self.live(actor) {
                self.touch_triggers(actor);
            }
            return trace;
        }
    }

    fn clip(&self, velocity: Vec3, normal: Vec3, overbounce: f64) -> Vec3 {
        let backoff = self.numeric.mul(self.dot(velocity, normal), overbounce);
        let clipped = self.vector(
            self.numeric
                .sub(f64::from(velocity.x), self.numeric.mul(f64::from(normal.x), backoff)),
            self.numeric
                .sub(f64::from(velocity.y), self.numeric.mul(f64::from(normal.y), backoff)),
            self.numeric
                .sub(f64::from(velocity.z), self.numeric.mul(f64::from(normal.z), backoff)),
        );
        let stop = |v: f32| if v > -0.1 && v < 0.1 { 0.0 } else { f64::from(v) };
        self.vector(stop(clipped.x), stop(clipped.y), stop(clipped.z))
    }

    fn test_position(&self, actor: &OwnedActor) -> Option<TraceResult> {
        let body = self.bodies.read(actor.id())?;
        let trace = self.body_trace(actor, body.origin, body.origin, &[], false, None);
        if trace.start_solid() {
            Some(trace)
        } else {
            None
        }
    }

    fn write_live(&mut self, actor: &OwnedActor, update: impl FnOnce(&mut BodyState), link: bool) -> bool {
        let Some(body) = self.bodies.read(actor.id()) else {
            return false;
        };
        let mut updated = body;
        update(&mut updated);
        self.bodies.write(actor, updated);
        if link {
            self.link_body(actor);
        }
        true
    }

    fn candidates(&self) -> Vec<OwnedActor> {
        let mut ids: Vec<ActorId> = self
            .actors
            .observations()
            .iter()
            .map(|actor| actor.id.clone())
            .collect();
        ids.sort_by(|a, b| (self.options.source_order)(a, b));
        ids.into_iter()
            .filter_map(|id| self.actors.resolve_owned(&id))
            .collect()
    }

    /// Read a Q1 pusher snapshot for one actor.
    pub fn read_q1_pusher(&self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
        let owned = self.actors.resolve_owned(actor)?;
        let body = self.bodies.read(actor)?;
        let linked = self.bodies.linked(actor)?;
        let flags = self.actor_flags(&owned);
        let motion = self.motion(&owned);
        let solid = self.solid(&owned);
        let move_type = if flags.player == Some(true) {
            3
        } else {
            match motion.as_ref().map(|motion| &motion.kind) {
                Some(Q2MotionKind::Push) | Some(Q2MotionKind::Stop) => 7,
                Some(Q2MotionKind::Step) => 4,
                Some(Q2MotionKind::Fly) => 5,
                Some(Q2MotionKind::FlyMissile) => 9,
                Some(Q2MotionKind::Bounce) | Some(Q2MotionKind::WallBounce) => 10,
                Some(Q2MotionKind::Toss) | Some(Q2MotionKind::NewToss) => 6,
                _ => 0,
            }
        };
        Some(Q1PhysicsEntity {
            actor: owned,
            state: Q1MovementState {
                origin: body.origin,
                velocity: body.velocity,
                angles: body.angles,
                old_origin: body.origin,
                angular_velocity: motion
                    .as_ref()
                    .map(|motion| motion.angular_velocity)
                    .unwrap_or_else(zero),
                view_angles: zero(),
                punch_angles: zero(),
                move_type,
                flags: (if body.ground.is_some() { 512 } else { 0 })
                    | (if flags.fly == Some(true) { 1 } else { 0 })
                    | (if flags.swim == Some(true) { 2 } else { 0 }),
                ground: match body.ground {
                    None => TraceHit::None,
                    Some(ground) => TraceHit::Actor { actor: ground },
                },
                water_level: flags.water_level.unwrap_or(0),
                water_type: flags.water_type.unwrap_or(-1),
                teleport_time_seconds: 0.0,
                water_jump_direction: zero(),
                ideal_pitch: 0.0,
                fix_angle: false,
                health: 0.0,
            },
            bounds: body.bounds,
            absolute_bounds: linked.absolute_bounds,
            solid: if solid.as_ref().is_some_and(|solid| solid.dead_monster == Some(true)) {
                Q1Solid::Corpse
            } else if solid.as_ref().is_some_and(|solid| solid.solid == SolidKind::Brush) {
                Q1Solid::Bsp
            } else if solid.as_ref().is_some_and(|solid| solid.solid == SolidKind::Trigger) {
                Q1Solid::Trigger
            } else if solid.as_ref().is_some_and(|solid| solid.solid == SolidKind::Box) {
                Q1Solid::Box
            } else {
                Q1Solid::Not
            },
            local_time_seconds: 0.0,
            next_think_seconds: 0.0,
        })
    }

    /// Write a Q1 pusher snapshot back to its body.
    pub fn write_q1_pusher(&mut self, entity: &Q1PhysicsEntity) {
        let ground = if entity.state.flags & 512 == 0 {
            None
        } else {
            match &entity.state.ground {
                TraceHit::Actor { actor } => Some(actor.clone()),
                TraceHit::World { .. } => (self.options.world_actor)(),
                TraceHit::None => None,
            }
        };
        let origin = entity.state.origin;
        let angles = entity.state.angles;
        let bounds = entity.bounds;
        self.write_live(
            &entity.actor,
            |body| {
                body.origin = origin;
                body.angles = angles;
                body.bounds = bounds;
                body.ground = ground;
            },
            false,
        );
    }

    fn push_q1(
        &mut self,
        actor: &OwnedActor,
        displacement: Vec3,
        angular_displacement: Vec3,
    ) -> Result<Option<ActorId>, PhysicsError> {
        let mut adapter = Q1PusherAdapter {
            physics: self,
            obstacle: None,
        };
        push_q1_pusher(
            &Q1PushInput {
                actor: actor.id().clone(),
                displacement,
                angular_displacement,
                elapsed_seconds: 0.0,
            },
            &mut adapter,
        )?;
        Ok(adapter.obstacle)
    }

    /// Move a pusher and its riders, returning the blocking actor if any.
    pub fn push_move(
        &mut self,
        actor: &OwnedActor,
        displacement: Vec3,
        angular_displacement: Vec3,
    ) -> Result<Option<ActorId>, PhysicsError> {
        let Some(original) = self.bodies.read(actor.id()) else {
            return Ok(None);
        };
        if self.family(actor) == PhysicsFamily::Q1 {
            return self.push_q1(actor, displacement, angular_displacement);
        }
        let snap = |v: f64| {
            self.numeric
                .store((v * 8.0 + if v > 0.0 { 0.5 } else { -0.5 }).trunc() * 0.125)
        };
        let displacement = self.vector(
            f64::from(snap(f64::from(displacement.x))),
            f64::from(snap(f64::from(displacement.y))),
            f64::from(snap(f64::from(displacement.z))),
        );
        if !Self::moving(displacement) && !Self::moving(angular_displacement) {
            return Ok(None);
        }
        let in_transaction = self.push_transaction.is_some();
        if self.push_transaction.is_none() {
            self.push_transaction = Some(Vec::new());
        }
        self.transaction_push(Pushed {
            actor: actor.clone(),
            origin: original.origin,
            angles: original.angles,
            delta_yaw: self.actor_flags(actor).delta_yaw.unwrap_or(0.0),
        });
        let moved_origin = self.add(original.origin, displacement);
        let moved_angles = self.add(original.angles, angular_displacement);
        self.write_live(
            actor,
            |body| {
                body.origin = moved_origin;
                body.angles = moved_angles;
            },
            true,
        );
        let Some(bounds) = self.bodies.linked(actor.id()).map(|linked| linked.absolute_bounds) else {
            return Ok(None);
        };
        let angles = self.scale(angular_displacement, -1.0);
        let axes = if matches!(self.options.numeric.arithmetic, Arithmetic::DonorBinary64(_)) {
            let (mut forward, mut right, mut up) = (zero(), zero(), zero());
            donor_angle_vectors(angles, Some(&mut forward), Some(&mut right), Some(&mut up));
            qa_core::math::AngleVectors { forward, right, up }
        } else {
            angle_vectors(angles)
        };
        for candidate in self.candidates() {
            if candidate.id() == actor.id() || !self.live(actor) {
                continue;
            }
            let (Some(body), Some(linked)) = (self.bodies.read(candidate.id()), self.bodies.linked(candidate.id()))
            else {
                continue;
            };
            let kind = self.motion(&candidate).map(|motion| motion.kind);
            if matches!(
                kind,
                Some(Q2MotionKind::Push) | Some(Q2MotionKind::Stop) | Some(Q2MotionKind::Stationary) | None
            ) {
                continue;
            }
            let rider = body.ground.as_ref().is_some_and(|ground| ground == actor.id());
            if !rider
                && (!bounds_intersect(&linked.absolute_bounds, &bounds) || self.test_position(&candidate).is_none())
            {
                continue;
            }
            let mut blocked = self
                .motion(actor)
                .is_some_and(|motion| motion.kind == Q2MotionKind::Stop)
                && !rider;
            if !blocked {
                self.transaction_push(Pushed {
                    actor: candidate.clone(),
                    origin: body.origin,
                    angles: body.angles,
                    delta_yaw: self.actor_flags(&candidate).delta_yaw.unwrap_or(0.0),
                });
                let Some(pusher_origin) = self.bodies.read(actor.id()).map(|body| body.origin) else {
                    return Ok(None);
                };
                let translated = self.add(body.origin, displacement);
                let offset = self.sub(translated, pusher_origin);
                let rotated = self.vector(
                    self.dot(offset, axes.forward),
                    -self.dot(offset, axes.right),
                    self.dot(offset, axes.up),
                );
                let delta = self.add(displacement, self.sub(rotated, offset));
                let origin = self.add(body.origin, delta);
                let ground = if rider { body.ground.clone() } else { None };
                self.write_live(
                    &candidate,
                    |body| {
                        body.origin = origin;
                        body.ground = ground;
                    },
                    false,
                );
                if self.actor_flags(&candidate).player == Some(true) {
                    let yaw = self.numeric.add(
                        self.actor_flags(&candidate).delta_yaw.unwrap_or(0.0),
                        f64::from(angular_displacement.y),
                    );
                    self.set_flags(
                        &candidate,
                        SharedPhysicsFlags {
                            delta_yaw: Some(yaw),
                            ..SharedPhysicsFlags::default()
                        },
                    );
                }
                if !self.live(actor) {
                    return Ok(None);
                }
                if !self.live(&candidate) {
                    continue;
                }
                blocked = self.test_position(&candidate).is_some();
                if !blocked {
                    self.link_body(&candidate);
                    continue;
                }
                if let Some(state) = self.bodies.read(candidate.id()) {
                    let origin = self.sub(state.origin, displacement);
                    self.write_live(&candidate, |body| body.origin = origin, false);
                }
                if self.test_position(&candidate).is_none() {
                    self.transaction_pop();
                    continue;
                }
            }
            if blocked {
                let entries = self.transaction_snapshot();
                for entry in entries.into_iter().rev() {
                    self.restore_entry(&entry.actor, entry.origin, entry.angles, entry.delta_yaw);
                }
                if self.live(actor) {
                    (self.options.on_blocked)(actor, candidate.id());
                }
                return Ok(Some(candidate.id().clone()));
            }
        }
        if !in_transaction {
            let saved = self.push_transaction.take().unwrap_or_default();
            for entry in saved.iter().rev() {
                if self.live(&entry.actor) {
                    self.touch_triggers(&entry.actor);
                }
            }
        }
        Ok(None)
    }

    /// `SV_Physics_Pusher` commits an entire source team before touching triggers or running thinks.
    pub fn push_team(&mut self, actors: &[OwnedActor], elapsed: f64) -> Result<Option<ActorId>, PhysicsError> {
        if self.push_transaction.is_some() {
            return Err(PhysicsError::NestedPushTeam);
        }
        self.push_transaction = Some(Vec::new());
        let mut blocked = None;
        for actor in actors {
            if !self.live(actor) {
                continue;
            }
            let (Some(body), Some(motion)) = (self.bodies.read(actor.id()), self.motion(actor)) else {
                continue;
            };
            let obstacle = self.push_move(
                actor,
                self.scale(body.velocity, elapsed),
                self.scale(motion.angular_velocity, elapsed),
            )?;
            if obstacle.is_some() {
                blocked = obstacle;
                break;
            }
        }
        let saved = self.push_transaction.take().unwrap_or_default();
        if blocked.is_none() {
            for entry in saved.iter().rev() {
                if self.live(&entry.actor) {
                    self.touch_triggers(&entry.actor);
                }
            }
        }
        Ok(blocked)
    }

    fn transaction_push(&mut self, entry: Pushed) {
        if let Some(saved) = self.push_transaction.as_mut() {
            saved.push(entry);
        }
    }

    fn transaction_pop(&mut self) {
        if let Some(saved) = self.push_transaction.as_mut() {
            saved.pop();
        }
    }

    fn transaction_snapshot(&self) -> Vec<Pushed> {
        self.push_transaction.clone().unwrap_or_default()
    }

    fn restore_entry(&mut self, actor: &OwnedActor, origin: Vec3, angles: Vec3, delta_yaw: f64) {
        let Some(current) = self.bodies.read(actor.id()) else {
            return;
        };
        let mut restored = current;
        restored.origin = origin;
        restored.angles = angles;
        self.bodies.write(actor, restored);
        if self.actor_flags(actor).player == Some(true) {
            self.set_flags(
                actor,
                SharedPhysicsFlags {
                    delta_yaw: Some(delta_yaw),
                    ..SharedPhysicsFlags::default()
                },
            );
        }
        self.link_body(actor);
    }

    fn check_bottom(&self, actor: &OwnedActor, body: &BodyState) -> bool {
        let min = self.add(body.origin, body.bounds.min);
        let max = self.add(body.origin, body.bounds.max);
        let direction = self
            .motion(actor)
            .map(|motion| motion.gravity_vector)
            .unwrap_or(vec3(0.0, 0.0, -1.0));
        let floor = if direction.z > 0.0 { max.z } else { min.z };
        let point = |x: f32, y: f32| {
            self.scene.trace(&TraceQuery {
                start: Vec3 { x, y, z: floor },
                end: Vec3 {
                    x,
                    y,
                    z: self.numeric.store(f64::from(floor) + f64::from(direction.z) * 36.0),
                },
                shape: TraceShape::Point,
                target: TraceTarget::World,
                policy: self.policy(self.family(actor), DEFAULT_MASK, Q1Move::NoMonsters),
                numeric: self.options.numeric,
                pass_actor: Some(actor.id().clone()),
            })
        };
        let middle = point((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
        if middle.fraction() == 1.0 {
            return false;
        }
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                let corner = point(x, y);
                if corner.fraction() == 1.0
                    || (f64::from(corner.end().z) - f64::from(middle.end().z)) * f64::from(direction.z) > 18.0
                {
                    return false;
                }
            }
        }
        true
    }

    /// Step one walker toward a yaw by a distance.
    pub fn walk_move(&mut self, actor: &OwnedActor, yaw: f64, distance: f64) -> bool {
        let Some(body) = self.bodies.read(actor.id()) else {
            return false;
        };
        let radians = yaw * std::f64::consts::PI * 2.0 / 360.0;
        let step = self.vector(radians.cos() * distance, radians.sin() * distance, 0.0);
        let flags = self.actor_flags(actor);
        let family = self.family(actor);
        if flags.fly == Some(true) || flags.swim == Some(true) {
            let enemy = flags.enemy.as_ref().and_then(|enemy| self.bodies.read(enemy));
            for attempt in 0..if enemy.is_none() { 1 } else { 2 } {
                let mut end = self.add(body.origin, step);
                if attempt == 0 {
                    if let Some(enemy) = enemy.as_ref() {
                        let dz = f64::from(body.origin.z) - f64::from(enemy.origin.z);
                        end = self.add(
                            end,
                            vec3(
                                0.0,
                                0.0,
                                if dz > 40.0 {
                                    -8.0
                                } else if dz < 30.0 {
                                    8.0
                                } else {
                                    0.0
                                },
                            ),
                        );
                    }
                }
                let trace = self.body_trace(actor, body.origin, end, &[], false, None);
                if trace.fraction() != 1.0 {
                    continue;
                }
                let contents = self.scene.point_contents(&PointContentsQuery {
                    point: trace.end(),
                    target: TraceTarget::World,
                    policy: self.policy(family, DEFAULT_MASK, Q1Move::Normal),
                    numeric: self.options.numeric,
                    pass_actor: Some(actor.id().clone()),
                });
                let water = match contents {
                    PointContents::Q1 { contents } => (-5..=-3).contains(&contents),
                    PointContents::Q2 { merged, .. } => merged & 56 != 0,
                    PointContents::Q3 { contents } => contents & 56 != 0,
                };
                if flags.swim == Some(true) && !water {
                    return false;
                }
                let end = trace.end();
                self.write_live(actor, |body| body.origin = end, true);
                self.touch_triggers(actor);
                return self.live(actor);
            }
            return false;
        }
        let desired = self.add(body.origin, step);
        let raised = self.add(desired, vec3(0.0, 0.0, 18.0));
        let down = self.add(desired, vec3(0.0, 0.0, -18.0));
        let mut trace = self.body_trace(actor, raised, down, &[], false, None);
        if trace.all_solid() {
            return false;
        }
        if trace.start_solid() {
            trace = self.body_trace(actor, desired, down, &[], false, None);
            if trace.all_solid() || trace.start_solid() {
                return false;
            }
        }
        if trace.fraction() == 1.0 {
            if flags.partial_ground != Some(true) {
                return false;
            }
            self.write_live(
                actor,
                |body| {
                    body.origin = desired;
                    body.ground = None;
                },
                true,
            );
            self.touch_triggers(actor);
            return self.live(actor);
        }
        let landed = BodyState {
            origin: trace.end(),
            ..body
        };
        if !self.check_bottom(actor, &landed) {
            if flags.partial_ground != Some(true) {
                return false;
            }
            let end = trace.end();
            self.write_live(actor, |body| body.origin = end, true);
            self.touch_triggers(actor);
            return self.live(actor);
        }
        let end = trace.end();
        let ground = self.hit_actor(&trace);
        self.write_live(
            actor,
            |body| {
                body.origin = end;
                body.ground = ground;
            },
            true,
        );
        self.set_flags(
            actor,
            SharedPhysicsFlags {
                partial_ground: Some(false),
                ..SharedPhysicsFlags::default()
            },
        );
        self.touch_triggers(actor);
        self.live(actor)
    }

    fn fly_move(&mut self, actor: &OwnedActor, elapsed: f64, new_toss: bool) -> Result<(), PhysicsError> {
        if self.family(actor) == PhysicsFamily::Q2 && self.options.q2_edition == Some(Q2Edition::Rerelease) {
            let fly = self.rerelease_fly_move;
            let mut context = std::mem::replace(&mut self.rerelease_movement, Q2RereleaseMovementContext::new());
            let result = {
                let mut host = RereleaseHost {
                    physics: self,
                    actor: actor.clone(),
                    new_toss,
                };
                fly.run(&mut context, actor, elapsed, &mut host)
            };
            self.rerelease_movement = context;
            return Ok(result?);
        }
        if self.bodies.read(actor.id()).is_none() {
            return Ok(());
        }
        self.write_live(actor, |body| body.ground = None, false);
        let mut sweep = FlySweep {
            physics: self,
            actor: actor.clone(),
            new_toss,
        };
        sweep_body(&mut sweep, elapsed);
        Ok(())
    }

    /// Sample contents at the actor origin and emit water transitions.
    pub fn water_transition(&mut self, actor: &OwnedActor, previous_origin: Vec3) {
        let Some(body) = self.bodies.read(actor.id()) else {
            return;
        };
        let family = self.family(actor);
        let before = self.actor_flags(actor);
        let contents = self.scene.point_contents(&PointContentsQuery {
            point: body.origin,
            target: TraceTarget::World,
            policy: self.policy(family, DEFAULT_MASK, Q1Move::Normal),
            numeric: self.options.numeric,
            pass_actor: Some(actor.id().clone()),
        });
        let value = match contents {
            PointContents::Q1 { contents } => contents,
            PointContents::Q2 { merged, .. } => merged,
            PointContents::Q3 { contents } => contents,
        };
        let wet = if family == PhysicsFamily::Q1 {
            (-5..=-3).contains(&value) || (-14..=-9).contains(&value)
        } else {
            value & 56 != 0
        };
        let was_wet = before.water_level.unwrap_or(0) != 0;
        self.set_flags(
            actor,
            SharedPhysicsFlags {
                water_level: Some(if wet { 1 } else { 0 }),
                water_type: Some(value),
                ..SharedPhysicsFlags::default()
            },
        );
        if wet != was_wet {
            self.emit(PhysicsEvent {
                actor: actor.id().clone(),
                kind: if wet {
                    PhysicsEventKind::WaterEnter
                } else {
                    PhysicsEventKind::WaterLeave
                },
                origin: if wet { previous_origin } else { body.origin },
            });
        }
    }

    /// Step physics for one actor over an interval in seconds.
    pub fn step(&mut self, actor: &OwnedActor, elapsed: f64) -> Result<StepOutcome, PhysicsError> {
        if !elapsed.is_finite() || elapsed < 0.0 {
            return Err(PhysicsError::InvalidInterval);
        }
        let Some(mut state) = self.bodies.read(actor.id()) else {
            return Ok(None);
        };
        let Some(motion) = self.motion(actor) else {
            return Ok(None);
        };
        if elapsed == 0.0 || motion.kind == Q2MotionKind::Stationary {
            return Ok(None);
        }
        if self.bodies.attachment(actor.id()).is_some() {
            if motion.kind == Q2MotionKind::Fly || motion.kind == Q2MotionKind::FlyMissile {
                let angles = self.add(state.angles, self.scale(motion.angular_velocity, elapsed));
                self.write_live(actor, |body| body.angles = angles, false);
            }
            self.push_entity(actor, zero(), &[]);
            if self.live(actor) {
                self.water_transition(actor, state.origin);
            }
            return Ok(None);
        }
        if motion.kind == Q2MotionKind::Push || motion.kind == Q2MotionKind::Stop {
            self.push_move(
                actor,
                self.scale(state.velocity, elapsed),
                self.scale(motion.angular_velocity, elapsed),
            )?;
            return Ok(None);
        }
        let family = self.family(actor);
        let flags = self.actor_flags(actor);
        if motion.kind == Q2MotionKind::NewToss {
            let motion_snapshot = motion.clone();
            let mut host = NewTossHost {
                physics: self,
                actor: actor.clone(),
                motion: motion_snapshot,
            };
            let outcome = step_q2_new_toss(actor, elapsed, &motion, &mut host);
            return Ok(Some(outcome));
        }
        if state.ground.is_some()
            && (state.ground.as_ref().is_some_and(|ground| !self.actors.is_live(ground))
                || (family != PhysicsFamily::Q1 && self.dot(state.velocity, motion.gravity_vector) < 0.0))
        {
            self.write_live(actor, |body| body.ground = None, false);
            if let Some(updated) = self.bodies.read(actor.id()) {
                state = updated;
            } else {
                return Ok(None);
            }
        }
        let state_value = state;
        let maximum = self.options.max_velocity.unwrap_or(2000.0);
        let clamp = |v: f32| {
            if !f64::from(v).is_finite() {
                0.0
            } else {
                f64::from(v).max(-maximum).min(maximum)
            }
        };
        let mut velocity = self.vector(
            clamp(state_value.velocity.x),
            clamp(state_value.velocity.y),
            clamp(state_value.velocity.z),
        );
        if motion.kind == Q2MotionKind::Step {
            return self.step_motion(actor, &state_value, &motion, &mut velocity, family, &flags, elapsed);
        }
        if state_value.ground.is_some() {
            return Ok(None);
        }
        if motion.kind != Q2MotionKind::Fly
            && motion.kind != Q2MotionKind::FlyMissile
            && motion.kind != Q2MotionKind::WallBounce
        {
            velocity = self.add(
                velocity,
                self.scale(motion.gravity_vector, motion.gravity * self.world_gravity * elapsed),
            );
        }
        let angles = self.add(state_value.angles, self.scale(motion.angular_velocity, elapsed));
        self.write_live(
            actor,
            |body| {
                body.velocity = velocity;
                body.angles = angles;
            },
            false,
        );
        let trace = self.push_entity(actor, self.scale(velocity, elapsed), &[]);
        if family != PhysicsFamily::Q1 {
            self.water_transition(actor, state_value.origin);
        }
        let Some(state) = self.bodies.read(actor.id()) else {
            return Ok(None);
        };
        if trace.fraction() == 1.0 {
            return Ok(None);
        }
        let current = self.motion(actor).unwrap_or(motion);
        let normal = trace.contact_normal().unwrap_or_else(|| trace.source_normal());
        velocity = self.clip(
            state.velocity,
            normal,
            if current.kind == Q2MotionKind::WallBounce {
                2.0
            } else if current.kind == Q2MotionKind::Bounce {
                1.5
            } else {
                1.0
            },
        );
        if current.kind != Q2MotionKind::WallBounce
            && self.dot(normal, current.gravity_vector) < -0.7
            && (self.dot(velocity, current.gravity_vector) > -60.0 || current.kind != Q2MotionKind::Bounce)
        {
            let ground = self.hit_actor(&trace);
            self.write_live(
                actor,
                |body| {
                    body.velocity = zero();
                    body.ground = ground;
                },
                false,
            );
            self.motions.insert(
                actor.id().clone(),
                Q2Motion {
                    velocity: zero(),
                    angular_velocity: zero(),
                    ..current
                },
            );
            self.write_angular_velocity(actor, zero());
        } else {
            self.write_live(actor, |body| body.velocity = velocity, false);
        }
        if family == PhysicsFamily::Q1 && self.live(actor) {
            if let Some(source) = self.sources.get_mut(actor.id()) {
                source.water_transition();
            } else if let Some(transition) = self.options.q1_water_transition.as_mut() {
                transition(actor);
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn step_motion(
        &mut self,
        actor: &OwnedActor,
        state_value: &BodyState,
        motion: &Q2Motion,
        velocity: &mut Vec3,
        family: PhysicsFamily,
        flags: &SharedPhysicsFlags,
        elapsed: f64,
    ) -> Result<StepOutcome, PhysicsError> {
        let maximum = self.options.max_velocity.unwrap_or(2000.0);
        let clamp = |v: f32| {
            if !f64::from(v).is_finite() {
                0.0
            } else {
                f64::from(v).max(-maximum).min(maximum)
            }
        };
        if family == PhysicsFamily::Q1 {
            if state_value.ground.is_some() || flags.fly == Some(true) || flags.swim == Some(true) {
                return Ok(None);
            }
            let hit_sound = f64::from(state_value.velocity.z) < -self.world_gravity * 0.1;
            *velocity = self.add(
                state_value.velocity,
                self.scale(motion.gravity_vector, motion.gravity * self.world_gravity * elapsed),
            );
            let clamped = self.vector(clamp(velocity.x), clamp(velocity.y), clamp(velocity.z));
            self.write_live(actor, |body| body.velocity = clamped, false);
            self.fly_move(actor, elapsed, false)?;
            if self.live(actor) {
                self.link_body(actor);
                self.touch_triggers(actor);
            }
            let landed = self.bodies.read(actor.id());
            if hit_sound && landed.as_ref().is_some_and(|landed| landed.ground.is_some()) {
                self.emit(PhysicsEvent {
                    actor: actor.id().clone(),
                    kind: PhysicsEventKind::Land,
                    origin: landed.expect("landed checked").origin,
                });
            }
            return Ok(None);
        }
        let mut state_value = state_value.clone();
        if state_value.ground.is_none() && self.dot(*velocity, motion.gravity_vector) >= -100.0 {
            let floor = self.body_trace(
                actor,
                state_value.origin,
                self.add(state_value.origin, self.scale(motion.gravity_vector, 0.25)),
                &[],
                false,
                None,
            );
            if floor.fraction() < 1.0
                && !floor.start_solid()
                && self.dot(floor.source_plane().normal, motion.gravity_vector) <= -0.7
            {
                let ground = self.hit_actor(&floor);
                self.write_live(actor, |body| body.ground = ground, false);
                if let Some(updated) = self.bodies.read(actor.id()) {
                    state_value = updated;
                }
            }
        }
        let was_grounded = state_value.ground.is_some();
        let falling_fast = self.dot(state_value.velocity, motion.gravity_vector) > self.world_gravity * 0.1;
        let angles = self.add(state_value.angles, self.scale(motion.angular_velocity, elapsed));
        self.write_live(actor, |body| body.angles = angles, false);
        if Self::moving(motion.angular_velocity) {
            let adjustment = elapsed * 600.0;
            let friction = |value: f32| {
                if value > 0.0 {
                    (f64::from(value) - adjustment).max(0.0)
                } else {
                    (f64::from(value) + adjustment).min(0.0)
                }
            };
            let angular = self.vector(
                friction(motion.angular_velocity.x),
                friction(motion.angular_velocity.y),
                friction(motion.angular_velocity.z),
            );
            self.motions.insert(
                actor.id().clone(),
                Q2Motion {
                    angular_velocity: angular,
                    ..motion.clone()
                },
            );
            self.write_angular_velocity(actor, angular);
        }
        if state_value.ground.is_none()
            && flags.fly != Some(true)
            && !(flags.swim == Some(true) && flags.water_level.unwrap_or(0) > 2)
            && flags.water_level.unwrap_or(0) == 0
        {
            *velocity = self.add(
                *velocity,
                self.scale(motion.gravity_vector, motion.gravity * self.world_gravity * elapsed),
            );
        }
        if flags.fly == Some(true) && velocity.z != 0.0 {
            let speed = f64::from(velocity.z).abs();
            let factor = (speed - elapsed * speed.max(100.0) * 2.0).max(0.0) / speed;
            *velocity = self.vector(
                f64::from(velocity.x),
                f64::from(velocity.y),
                f64::from(velocity.z) * factor,
            );
        }
        if flags.swim == Some(true) && velocity.z != 0.0 {
            let speed = f64::from(velocity.z).abs();
            let factor =
                (speed - elapsed * speed.max(100.0) * f64::from(flags.water_level.unwrap_or(0))).max(0.0) / speed;
            *velocity = self.vector(
                f64::from(velocity.x),
                f64::from(velocity.y),
                f64::from(velocity.z) * factor,
            );
        }
        if Self::moving(*velocity) {
            if state_value.ground.is_some() || flags.fly == Some(true) || flags.swim == Some(true) {
                let speed = (f64::from(velocity.x).powi(2) + f64::from(velocity.y).powi(2)).sqrt();
                if speed > 0.0 && (flags.dead != Some(true) || self.check_bottom(actor, &state_value)) {
                    let fraction = (speed - elapsed * speed.max(100.0) * 6.0).max(0.0) / speed;
                    *velocity = self.vector(
                        f64::from(velocity.x) * fraction,
                        f64::from(velocity.y) * fraction,
                        f64::from(velocity.z),
                    );
                }
            }
            let stepped = *velocity;
            self.write_live(actor, |body| body.velocity = stepped, false);
            self.fly_move(actor, elapsed, false)?;
            if self.live(actor) {
                self.link_body(actor);
                self.touch_triggers(actor);
            }
            let landed = self.bodies.read(actor.id());
            if !was_grounded && falling_fast && landed.as_ref().is_some_and(|landed| landed.ground.is_some()) {
                self.emit(PhysicsEvent {
                    actor: actor.id().clone(),
                    kind: PhysicsEventKind::Land,
                    origin: landed.expect("landed checked").origin,
                });
            }
        } else {
            let settled = *velocity;
            self.write_live(actor, |body| body.velocity = settled, false);
        }
        Ok(None)
    }

    /// Transport attachments at a committed execution boundary.
    pub fn commit_attachments(&mut self) {
        self.bodies.transport_attachments();
    }
}

struct Q1PusherAdapter<'p> {
    physics: &'p mut SharedPhysics,
    obstacle: Option<ActorId>,
}

impl Q1PusherServices for Q1PusherAdapter<'_> {
    fn numeric(&self) -> NumericOps {
        self.physics.numeric
    }

    fn read(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
        self.physics.read_q1_pusher(actor)
    }

    fn candidates(&mut self) -> Vec<ActorId> {
        self.physics
            .candidates()
            .iter()
            .map(|actor| actor.id().clone())
            .collect()
    }

    fn write(&mut self, entity: Q1PhysicsEntity) {
        self.physics.write_q1_pusher(&entity);
    }

    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool) {
        self.physics.link_body(actor);
        if touch_triggers {
            self.physics.touch_triggers(actor);
        }
    }

    fn collision_enabled(&mut self, actor: &OwnedActor, enabled: bool) {
        if enabled {
            self.physics.link_body(actor);
        } else {
            self.physics.scene.unlink(actor.id());
        }
    }

    fn test_position(&mut self, entity: &Q1PhysicsEntity) -> TraceHit {
        self.physics
            .test_position(&entity.actor)
            .map(|trace| trace.hit().clone())
            .unwrap_or(TraceHit::None)
    }

    fn push(&mut self, entity: &Q1PhysicsEntity, displacement: Vec3) -> (Option<Q1PhysicsEntity>, Q1Trace) {
        // Publish cleared onground before the synchronous source touch callback.
        self.physics.write_q1_pusher(entity);
        let trace = self.physics.push_entity(&entity.actor, displacement, &[]);
        (self.physics.read_q1_pusher(entity.actor.id()), to_q1_trace(&trace))
    }

    fn blocked(&mut self, pusher: &OwnedActor, obstacle: &ActorId) {
        self.obstacle = Some(obstacle.clone());
        (self.physics.options.on_blocked)(pusher, obstacle);
    }
}

/// Project a scene trace onto the Q1 pusher view.
///
/// Riders in a Q1 pusher transaction may belong to another family; the
/// pusher only reads the common travel fields, which this preserves exactly.
fn to_q1_trace(trace: &TraceResult) -> Q1Trace {
    if let TraceResult::Q1(q1) = trace {
        return q1.clone();
    }
    Q1Trace {
        fraction: trace.fraction(),
        end: trace.end(),
        start_solid: trace.start_solid(),
        all_solid: trace.all_solid(),
        contact: *trace.contact(),
        hit: trace.hit().clone(),
        in_open: false,
        in_water: false,
        source_plane: trace.source_plane(),
        surface_flags: None,
    }
}

struct FlySweep<'p> {
    physics: &'p mut SharedPhysics,
    actor: OwnedActor,
    new_toss: bool,
}

impl SweptBodyServices for FlySweep<'_> {
    type Trace = TraceResult;

    fn read(&mut self) -> Option<SweptBodyState> {
        if !self.physics.live(&self.actor) {
            return None;
        }
        self.physics.bodies.read(self.actor.id()).map(|body| SweptBodyState {
            origin: body.origin,
            velocity: body.velocity,
        })
    }

    fn write_origin(&mut self, origin: Vec3) {
        self.physics.write_live(&self.actor, |body| body.origin = origin, false);
    }

    fn write_velocity(&mut self, velocity: Vec3) {
        self.physics
            .write_live(&self.actor, |body| body.velocity = velocity, false);
    }

    fn trace(&mut self, start: Vec3, end: Vec3) -> TraceResult {
        let actor = self.actor.clone();
        self.physics.body_trace(&actor, start, end, &[], self.new_toss, None)
    }

    fn normal(&mut self, trace: &TraceResult) -> Vec3 {
        trace.contact_normal().unwrap_or_else(|| trace.source_normal())
    }

    fn impact(&mut self, trace: &TraceResult, normal: Vec3) {
        let actor = self.actor.clone();
        let hit = self.physics.hit_actor(trace);
        let target = hit.as_ref().and_then(|hit| self.physics.actors.resolve_owned(hit));
        let down = self
            .physics
            .motion(&actor)
            .map(|motion| motion.gravity_vector)
            .unwrap_or(vec3(0.0, 0.0, -1.0));
        let grounded = if self.new_toss {
            normal.z > 0.7
        } else {
            self.physics.dot(normal, down) < -0.7
        };
        if grounded
            && hit.is_some()
            && (matches!(trace.hit(), TraceHit::World { .. })
                || target.as_ref().is_some_and(|target| {
                    self.physics
                        .solid(target)
                        .is_some_and(|solid| solid.solid == SolidKind::Brush)
                }))
        {
            let ground = hit;
            self.physics.write_live(&actor, |body| body.ground = ground, false);
        }
        self.physics.impact(&actor, trace);
    }

    fn stop_when_still(&self) -> bool {
        false
    }

    fn same_plane(&mut self, first: Vec3, second: Vec3) -> bool {
        // The donor compares Q1 planes by reference and every sweep trace
        // carries fresh objects, so only non-Q1 families compare values.
        self.physics.family(&self.actor) != PhysicsFamily::Q1 && first == second
    }

    fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3 {
        let physics = &*self.physics;
        physics.add(origin, physics.scale(velocity, time))
    }

    fn remaining(&mut self, time: f64, fraction: f64) -> f64 {
        self.physics.numeric.sub(time, self.physics.numeric.mul(time, fraction))
    }

    fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3 {
        let physics = &*self.physics;
        physics.clip(velocity, normal, 1.0)
    }

    fn dot(&mut self, first: Vec3, second: Vec3) -> f64 {
        self.physics.dot(first, second)
    }

    fn cross(&mut self, a: Vec3, b: Vec3) -> Vec3 {
        self.physics.vector(
            f64::from(a.y) * f64::from(b.z) - f64::from(a.z) * f64::from(b.y),
            f64::from(a.z) * f64::from(b.x) - f64::from(a.x) * f64::from(b.z),
            f64::from(a.x) * f64::from(b.y) - f64::from(a.y) * f64::from(b.x),
        )
    }

    fn scale(&mut self, vector: Vec3, amount: f64) -> Vec3 {
        self.physics.scale(vector, amount)
    }
}

struct RereleaseHost<'p> {
    physics: &'p mut SharedPhysics,
    actor: OwnedActor,
    new_toss: bool,
}

impl RereleaseFlyMoveServices for RereleaseHost<'_> {
    fn is_live(&self, actor: &ActorId) -> bool {
        self.physics.actors.is_live(actor)
    }

    fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
        self.physics.bodies.read(actor)
    }

    fn write_body(&mut self, actor: &OwnedActor, state: BodyState) {
        self.physics.bodies.write(actor, state);
    }

    fn trace(&mut self, start: Vec3, end: Vec3, bounds: Bounds) -> TraceResult {
        let actor = self.actor.clone();
        let new_toss = self.new_toss;
        self.physics.body_trace(&actor, start, end, &[], new_toss, Some(bounds))
    }

    fn hit_actor(&self, trace: &TraceResult) -> Option<ActorId> {
        self.physics.hit_actor(trace)
    }

    fn impact(&mut self, trace: &TraceResult) {
        let actor = self.actor.clone();
        self.physics.impact(&actor, trace);
    }

    fn take_kill_velocity(&mut self) -> bool {
        let actor = self.actor.clone();
        self.physics
            .options
            .take_kill_velocity
            .as_mut()
            .is_some_and(|take| take(&actor))
    }
}

struct NewTossHost<'p> {
    physics: &'p mut SharedPhysics,
    actor: OwnedActor,
    motion: Q2Motion,
}

impl NewTossServices for NewTossHost<'_> {
    fn numeric(&self) -> NumericOps {
        self.physics.numeric
    }

    fn edition(&self) -> NewTossEdition {
        if self.physics.options.q2_edition == Some(Q2Edition::Rerelease) {
            NewTossEdition::Rerelease
        } else {
            NewTossEdition::Classic
        }
    }

    fn world_gravity(&self) -> f64 {
        self.physics.world_gravity
    }

    fn max_velocity(&self) -> f64 {
        self.physics.options.max_velocity.unwrap_or(2000.0)
    }

    fn stop_speed(&self) -> f64 {
        self.physics
            .options
            .stop_speed
            .as_ref()
            .map(|stop| stop())
            .unwrap_or(100.0)
    }

    fn team_slave(&self) -> bool {
        self.physics.actor_flags(&self.actor).team_slave.unwrap_or(false)
    }

    fn is_live(&self, actor: &ActorId) -> bool {
        self.physics.actors.is_live(actor)
    }

    fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
        self.physics.bodies.read(actor)
    }

    fn write_body(&mut self, actor: &OwnedActor, state: BodyState) {
        self.physics.bodies.write(actor, state);
    }

    fn link_body(&mut self, actor: &OwnedActor) {
        let actor = actor.clone();
        self.physics.link_body(&actor);
    }

    fn water(&self) -> NewTossWater {
        let flags = self.physics.actor_flags(&self.actor);
        NewTossWater {
            level: flags.water_level.unwrap_or(0),
            water_type: flags.water_type.unwrap_or(0),
        }
    }

    fn trace(&mut self, start: Vec3, end: Vec3) -> TraceResult {
        let actor = self.actor.clone();
        self.physics.body_trace(&actor, start, end, &[], true, None)
    }

    fn hit_actor(&self, trace: &TraceResult) -> Option<ActorId> {
        match trace.hit() {
            TraceHit::Actor { actor } => Some(actor.clone()),
            _ => (self.physics.options.world_actor)(),
        }
    }

    fn fly_move(&mut self, elapsed: f64) {
        let actor = self.actor.clone();
        let _ = self.physics.fly_move(&actor, elapsed, true);
    }

    fn write_angular_velocity(&mut self, velocity: Vec3) {
        let actor = self.actor.clone();
        let motion = self.motion.clone();
        self.physics.motions.insert(
            actor.id().clone(),
            Q2Motion {
                angular_velocity: velocity,
                ..motion
            },
        );
        self.physics.write_angular_velocity(&actor, velocity);
    }

    fn touch_triggers(&mut self) {
        let actor = self.actor.clone();
        self.physics.touch_triggers(&actor);
    }

    fn point_contents(&self, origin: Vec3) -> i32 {
        let contents = self.physics.scene.point_contents(&PointContentsQuery {
            point: origin,
            target: TraceTarget::World,
            policy: self.physics.policy(PhysicsFamily::Q2, -1, Q1Move::Normal),
            numeric: self.physics.options.numeric,
            pass_actor: Some(self.actor.id().clone()),
        });
        match contents {
            PointContents::Q2 { stored, .. } => stored,
            _ => panic!("new-toss requires Q2 contents representation"),
        }
    }

    fn write_water(&mut self, level: i32, water_type: i32) {
        let actor = self.actor.clone();
        self.physics.set_flags(
            &actor,
            SharedPhysicsFlags {
                water_level: Some(level),
                water_type: Some(water_type),
                ..SharedPhysicsFlags::default()
            },
        );
    }

    fn water_sound(&mut self, origin: Vec3) {
        let actor = (self.physics.options.world_actor)().unwrap_or_else(|| self.actor.id().clone());
        self.physics.emit(PhysicsEvent {
            actor,
            kind: PhysicsEventKind::WaterEnter,
            origin,
        });
    }
}

fn merge_flags(into: &mut SharedPhysicsFlags, extra: &SharedPhysicsFlags) {
    if extra.team_slave.is_some() {
        into.team_slave = extra.team_slave;
    }
    if extra.always_touch.is_some() {
        into.always_touch = extra.always_touch;
    }
    if extra.fly.is_some() {
        into.fly = extra.fly;
    }
    if extra.swim.is_some() {
        into.swim = extra.swim;
    }
    if extra.partial_ground.is_some() {
        into.partial_ground = extra.partial_ground;
    }
    if extra.dead.is_some() {
        into.dead = extra.dead;
    }
    if extra.player.is_some() {
        into.player = extra.player;
    }
    if extra.water_level.is_some() {
        into.water_level = extra.water_level;
    }
    if extra.water_type.is_some() {
        into.water_type = extra.water_type;
    }
    if extra.enemy.is_some() {
        into.enemy = extra.enemy.clone();
    }
    if extra.delta_yaw.is_some() {
        into.delta_yaw = extra.delta_yaw;
    }
}

fn optional_bool(reader: &SaveReader, name: &str) -> Result<Option<bool>, WorldError> {
    let field = reader.field(name);
    if field.is_missing() {
        Ok(None)
    } else {
        Ok(Some(field.boolean()?))
    }
}

fn optional_number(reader: &SaveReader, name: &str) -> Result<Option<f64>, WorldError> {
    let field = reader.field(name);
    if field.is_missing() {
        Ok(None)
    } else {
        Ok(Some(field.number()?))
    }
}

fn read_corpse(reader: &SaveReader) -> Result<bool, WorldError> {
    let corpse = reader.field("q1Corpse");
    if corpse.is_missing() {
        return Ok(false);
    }
    if !corpse.boolean()? {
        return Err(corpse.fail("Saved Q1 corpse flag must be true or absent"));
    }
    Ok(true)
}

fn write_collision(collision: &ActorCollision) -> SaveJson {
    let mut members = vec![
        (
            "family",
            save_str(match collision.family {
                CollisionFamily::Q1 => "q1",
                CollisionFamily::Q2 => "q2",
                CollisionFamily::Q3 => "q3",
            }),
        ),
        (
            "shape",
            match collision.shape {
                CollisionShape::Box => obj(vec![("kind", save_str("box"))]),
                CollisionShape::Capsule => obj(vec![("kind", save_str("capsule"))]),
                CollisionShape::Model(model) => {
                    obj(vec![("kind", save_str("model")), ("model", int(i64::from(model)))])
                }
            },
        ),
        ("contents", int(i64::from(collision.contents))),
        (
            "owner",
            collision
                .owner
                .as_ref()
                .map(SavedActorId::from)
                .map_or(SaveJson::Null, write_saved_actor),
        ),
        (
            "role",
            save_str(if collision.role == CollisionRole::Trigger {
                "trigger"
            } else {
                "solid"
            }),
        ),
        ("monster", boolean(collision.monster)),
        ("deadMonster", boolean(collision.dead_monster)),
    ];
    if collision.q1_corpse {
        members.push(("q1Corpse", boolean(true)));
    }
    if let Some(q3_owner) = collision.q3_owner {
        members.push((
            "q3Owner",
            obj(vec![
                ("entityNumber", int(i64::from(q3_owner.entity_number))),
                ("ownerNumber", int(i64::from(q3_owner.owner_number))),
            ]),
        ));
    }
    obj(members)
}

fn write_solid(actor: &ActorId, solid: &SharedSolid) -> SaveJson {
    let mut members = vec![
        ("actor", write_saved_actor(SavedActorId::from(actor))),
        (
            "solid",
            save_str(match solid.solid {
                SolidKind::None => "none",
                SolidKind::Trigger => "trigger",
                SolidKind::Box => "box",
                SolidKind::Brush => "brush",
            }),
        ),
        (
            "model",
            solid.model.map_or(SaveJson::Null, |model| int(i64::from(model))),
        ),
        (
            "family",
            save_str(match solid.family {
                PhysicsFamily::Q1 => "q1",
                PhysicsFamily::Q2 => "q2",
                PhysicsFamily::Q3 => "q3",
            }),
        ),
        (
            "owner",
            solid
                .owner
                .as_ref()
                .map(SavedActorId::from)
                .map_or(SaveJson::Null, write_saved_actor),
        ),
    ];
    if let Some(monster) = solid.monster {
        members.push(("monster", boolean(monster)));
    }
    if let Some(dead) = solid.dead_monster {
        members.push(("deadMonster", boolean(dead)));
    }
    if solid.q1_corpse {
        members.push(("q1Corpse", boolean(true)));
    }
    if let Some(item) = solid.item {
        members.push(("item", boolean(item)));
    }
    obj(members)
}

fn write_motion(motion: &Q2Motion) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(SavedActorId::from(motion.actor.id()))),
        ("kind", save_str(motion_kind_name(&motion.kind))),
        ("velocity", write_vector(motion.velocity)),
        ("angularVelocity", write_vector(motion.angular_velocity)),
        ("gravity", num(motion.gravity)),
        ("gravityVector", write_vector(motion.gravity_vector)),
        ("clipMask", int(i64::from(motion.clip_mask))),
        (
            "owner",
            motion
                .owner
                .as_ref()
                .map(SavedActorId::from)
                .map_or(SaveJson::Null, write_saved_actor),
        ),
    ])
}

fn write_flags(actor: &ActorId, flags: &SharedPhysicsFlags) -> SaveJson {
    let mut members = vec![("actor", write_saved_actor(SavedActorId::from(actor)))];
    if let Some(team_slave) = flags.team_slave {
        members.push(("teamSlave", boolean(team_slave)));
    }
    if let Some(always_touch) = flags.always_touch {
        members.push(("alwaysTouch", boolean(always_touch)));
    }
    if let Some(fly) = flags.fly {
        members.push(("fly", boolean(fly)));
    }
    if let Some(swim) = flags.swim {
        members.push(("swim", boolean(swim)));
    }
    if let Some(partial) = flags.partial_ground {
        members.push(("partialGround", boolean(partial)));
    }
    if let Some(dead) = flags.dead {
        members.push(("dead", boolean(dead)));
    }
    if let Some(player) = flags.player {
        members.push(("player", boolean(player)));
    }
    if let Some(level) = flags.water_level {
        members.push(("waterLevel", int(i64::from(level))));
    }
    if let Some(kind) = flags.water_type {
        members.push(("waterType", int(i64::from(kind))));
    }
    if let Some(yaw) = flags.delta_yaw {
        members.push(("deltaYaw", num(yaw)));
    }
    if let Some(enemy) = flags.enemy.as_ref() {
        members.push(("enemy", write_saved_actor(SavedActorId::from(enemy))));
    }
    obj(members)
}

#[cfg(test)]
pub mod fakes {
    //! Test doubles over the physics seams, shared by the actor-cluster tests.
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::Q2_DONOR_PROFILE;
    use qa_world::movement::q2::types::Q2TracePlane;
    use qa_world::registry::ActorObservation;

    use super::*;

    /// Minting fake actor registry.
    pub struct FakeActors {
        /// Identity mint.
        pub identities: IdentityOwner,
        /// Live set, shared so callbacks can release mid-dispatch.
        pub live: Rc<RefCell<HashSet<ActorId>>>,
        provider: ProviderId,
        owned: HashMap<ActorId, OwnedActor>,
        next_slot: u32,
    }

    impl FakeActors {
        /// Fresh registry.
        pub fn new() -> Self {
            Self {
                identities: IdentityOwner::create("physics-test").expect("owner"),
                live: Rc::new(RefCell::new(HashSet::new())),
                provider: ProviderId::new("q2", "test"),
                owned: HashMap::new(),
                next_slot: 1,
            }
        }

        /// Mint a live actor.
        pub fn mint(&mut self) -> OwnedActor {
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.identities.actor(slot, 0);
            let owned = self.identities.owned_actor(&id, self.provider.clone()).expect("owned");
            self.live.borrow_mut().insert(id.clone());
            self.owned.insert(id, owned.clone());
            owned
        }

        /// Release an actor.
        pub fn kill(&self, actor: &ActorId) {
            self.live.borrow_mut().remove(actor);
        }

        fn is_live_inner(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }
    }

    impl Default for FakeActors {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PhysicsActors for FakeActors {
        fn is_live(&self, actor: &ActorId) -> bool {
            self.is_live_inner(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            if self.is_live_inner(actor) {
                self.owned.get(actor).cloned()
            } else {
                None
            }
        }

        fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            let live = self.live.borrow();
            self.owned
                .values()
                .find(|owned| {
                    owned.id().slot() == saved.slot
                        && owned.id().generation() == saved.generation
                        && live.contains(owned.id())
                })
                .cloned()
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.identities.actor(saved.slot, saved.generation)
        }

        fn observations(&self) -> Vec<ActorObservation> {
            let live = self.live.borrow();
            let mut observations: Vec<ActorObservation> = self
                .owned
                .values()
                .filter(|owned| live.contains(owned.id()))
                .map(|owned| ActorObservation {
                    id: owned.id().clone(),
                    owner: owned.owner().clone(),
                    definition: "q2:test/entity".to_string(),
                })
                .collect();
            observations.sort_by_key(|observation| (observation.id.slot(), observation.id.generation()));
            observations
        }

        fn assert_owned(&self, actor: &OwnedActor) {
            assert!(self.is_live_inner(actor.id()), "actor is not live");
        }
    }

    /// Map-backed fake body table.
    pub struct FakeBodies {
        /// Body states, shared for post-step assertions.
        pub states: Rc<RefCell<HashMap<ActorId, BodyState>>>,
        /// Linked snapshots, shared for post-step assertions.
        pub linked: Rc<RefCell<HashMap<ActorId, LinkedBody>>>,
        attachments: Rc<RefCell<HashMap<ActorId, BodyAttachment>>>,
        link_counts: Rc<RefCell<HashMap<ActorId, u64>>>,
    }

    impl FakeBodies {
        /// Fresh table.
        pub fn new() -> Self {
            Self {
                states: Rc::new(RefCell::new(HashMap::new())),
                linked: Rc::new(RefCell::new(HashMap::new())),
                attachments: Rc::new(RefCell::new(HashMap::new())),
                link_counts: Rc::new(RefCell::new(HashMap::new())),
            }
        }

        /// Attach a body to an anchor.
        pub fn attach(&self, actor: &ActorId, attachment: BodyAttachment) {
            self.attachments.borrow_mut().insert(actor.clone(), attachment);
        }
    }

    impl Default for FakeBodies {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PhysicsBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.states.borrow().get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn link(&mut self, actor: &OwnedActor, absolute_bounds: Bounds) {
            let mut counts = self.link_counts.borrow_mut();
            let count = counts.entry(actor.id().clone()).or_insert(0);
            *count += 1;
            let count = *count;
            drop(counts);
            if let Some(state) = self.states.borrow().get(actor.id()).cloned() {
                self.linked.borrow_mut().insert(
                    actor.id().clone(),
                    LinkedBody {
                        actor: actor.id().clone(),
                        state,
                        absolute_bounds,
                        link_count: count,
                    },
                );
            }
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.linked.borrow().get(actor).cloned()
        }

        fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment> {
            self.attachments.borrow().get(actor).cloned()
        }

        fn transport_attachments(&mut self) {}
    }

    /// Scripted fake scene.
    pub struct FakeScene {
        /// Queued trace results, popped per sweep.
        pub traces: Rc<RefCell<VecDeque<TraceResult>>>,
        /// Contents answer for every sample.
        pub contents: PointContents,
        /// Linked spatial entries.
        pub actors: RefCell<Vec<SpatialActor>>,
        /// Recorded links.
        pub links: Rc<RefCell<Vec<(LinkedBody, ActorCollision)>>>,
        /// Recorded unlinks.
        pub unlinks: Rc<RefCell<Vec<ActorId>>>,
        /// Scripted Q1 trigger touches.
        pub q1_touches: Vec<PhysicsTouch>,
    }

    impl FakeScene {
        /// Fresh scene answering dry contents.
        pub fn new() -> Self {
            Self {
                traces: Rc::new(RefCell::new(VecDeque::new())),
                contents: PointContents::Q2 { stored: 0, merged: 0 },
                actors: RefCell::new(Vec::new()),
                links: Rc::new(RefCell::new(Vec::new())),
                unlinks: Rc::new(RefCell::new(Vec::new())),
                q1_touches: Vec::new(),
            }
        }

        fn clear(query: &TraceQuery) -> TraceResult {
            TraceResult::Q2(Q2Trace {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                contents: 0,
                surface: None,
                source_plane: Q2TracePlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    dist: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
                secondary: None,
            })
        }
    }

    impl Default for FakeScene {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PhysicsScene for FakeScene {
        fn trace(&self, query: &TraceQuery) -> TraceResult {
            self.traces
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Self::clear(query))
        }

        fn trace_excluding(&self, query: &TraceQuery, _exclude: &[ActorId]) -> TraceResult {
            self.trace(query)
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContents {
            self.contents
        }

        fn query_actors(&self, _bounds: &Bounds, role: QueryRole) -> Vec<SpatialActor> {
            self.actors
                .borrow()
                .iter()
                .filter(|entry| {
                    role == QueryRole::Both
                        || (role == QueryRole::Trigger && entry.collision.role == CollisionRole::Trigger)
                        || (role == QueryRole::Solid && entry.collision.role == CollisionRole::Solid)
                })
                .cloned()
                .collect()
        }

        fn link(&mut self, body: &LinkedBody, collision: &ActorCollision) {
            self.links.borrow_mut().push((body.clone(), collision.clone()));
            let mut actors = self.actors.borrow_mut();
            if let Some(entry) = actors.iter_mut().find(|entry| entry.body.actor == body.actor) {
                entry.body = body.clone();
                entry.collision = collision.clone();
            } else {
                actors.push(SpatialActor {
                    body: body.clone(),
                    collision: collision.clone(),
                });
            }
        }

        fn unlink(&mut self, actor: &ActorId) {
            self.unlinks.borrow_mut().push(actor.clone());
            self.actors.borrow_mut().retain(|entry| entry.body.actor != *actor);
        }

        fn spatial_snapshot(&self) -> Vec<SpatialActor> {
            self.actors.borrow().clone()
        }

        fn spatial_clear(&mut self) {
            self.actors.borrow_mut().clear();
        }

        fn touch_q1_triggers(&mut self, _mover: &OwnedActor, touch: &mut dyn FnMut(PhysicsTouch)) {
            for contact in std::mem::take(&mut self.q1_touches) {
                touch(contact);
            }
        }
    }

    /// Recording fake callbacks.
    pub struct FakeCallbacks {
        /// Dispatched touches.
        pub touches: Rc<RefCell<Vec<PhysicsTouch>>>,
    }

    impl FakeCallbacks {
        /// Fresh callbacks.
        pub fn new() -> Self {
            Self {
                touches: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl Default for FakeCallbacks {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PhysicsCallbacks for FakeCallbacks {
        fn touch(&mut self, contact: PhysicsTouch) {
            self.touches.borrow_mut().push(contact);
        }
    }

    /// Default test body.
    pub fn test_body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 10.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            ground: None,
        }
    }

    /// Default test motion.
    pub fn test_motion(actor: &OwnedActor, kind: Q2MotionKind) -> Q2Motion {
        Q2Motion {
            actor: actor.clone(),
            velocity: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            kind,
            gravity: 1.0,
            gravity_vector: vec3(0.0, 0.0, -1.0),
            clip_mask: 0,
            owner: None,
        }
    }

    /// Clear Q2 trace ending where asked.
    pub fn clear_q2_trace(end: Vec3) -> TraceResult {
        TraceResult::Q2(Q2Trace {
            fraction: 1.0,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface: None,
            source_plane: Q2TracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                dist: 0.0,
                plane_type: 0,
                signbits: 0,
            },
            secondary: None,
        })
    }

    /// Default test options.
    pub fn test_options(world: Option<ActorId>) -> SharedPhysicsOptions {
        SharedPhysicsOptions {
            numeric: Q2_DONOR_PROFILE,
            source_order: Box::new(|a, b| (a.slot(), a.generation()).cmp(&(b.slot(), b.generation()))),
            world_actor: Box::new(move || world.clone()),
            on_blocked: Box::new(|_, _| {}),
            get_collision: None,
            get_motion: None,
            get_flags: None,
            write_flags: None,
            write_angular_velocity: None,
            event: None,
            gravity: None,
            max_velocity: None,
            q2_edition: None,
            stop_speed: None,
            take_kill_velocity: None,
            q1_water_transition: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;

    use qa_world::movement::q2::types::{Q2Trace, Q2TracePlane};

    use super::fakes::*;
    use super::*;

    struct Handles {
        states: Rc<RefCell<HashMap<ActorId, BodyState>>>,
        traces: Rc<RefCell<VecDeque<TraceResult>>>,
        touches: Rc<RefCell<Vec<PhysicsTouch>>>,
    }

    fn with_rig(mint: usize, world_index: Option<usize>, run: impl FnOnce(Vec<OwnedActor>, SharedPhysics, Handles)) {
        let mut actors = FakeActors::new();
        let minted: Vec<OwnedActor> = (0..mint).map(|_| actors.mint()).collect();
        let world = world_index.map(|index| minted[index].id().clone());
        let bodies = FakeBodies::new();
        let scene = FakeScene::new();
        let callbacks = FakeCallbacks::new();
        let handles = Handles {
            states: bodies.states.clone(),
            traces: scene.traces.clone(),
            touches: callbacks.touches.clone(),
        };
        let physics = SharedPhysics::new(
            test_options(world),
            Box::new(actors),
            Box::new(bodies),
            Box::new(scene),
            Box::new(callbacks),
        )
        .expect("physics");
        run(minted, physics, handles);
    }

    fn floor_q2_trace(end: Vec3) -> TraceResult {
        TraceResult::Q2(Q2Trace {
            fraction: 0.5,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::Plane(Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 10.0,
            }),
            hit: TraceHit::World { model: 0 },
            contents: 1,
            surface: None,
            source_plane: Q2TracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                dist: 10.0,
                plane_type: 2,
                signbits: 0,
            },
            secondary: None,
        })
    }

    #[test]
    fn stationary_step_settles() {
        with_rig(1, None, |actors, mut physics, handles| {
            let actor = &actors[0];
            handles.states.borrow_mut().insert(actor.id().clone(), test_body());
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.set_motion(test_motion(actor, Q2MotionKind::Stationary));
            assert_eq!(physics.step(actor, 0.1).expect("step"), None);
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert_eq!(body.velocity, vec3(0.0, 0.0, 0.0));
        });
    }

    #[test]
    fn invalid_interval_errors() {
        with_rig(1, None, |actors, mut physics, _| {
            assert!(matches!(
                physics.step(&actors[0], -1.0),
                Err(PhysicsError::InvalidInterval)
            ));
            assert!(matches!(
                physics.step(&actors[0], f64::NAN),
                Err(PhysicsError::InvalidInterval)
            ));
        });
    }

    #[test]
    fn gravity_rejects_nonfinite() {
        with_rig(0, None, |_, mut physics, _| {
            assert!(matches!(
                physics.set_world_gravity(f64::INFINITY),
                Err(PhysicsError::InvalidGravity)
            ));
            physics.set_world_gravity(900.0).expect("gravity");
            assert_eq!(physics.gravity(), 900.0);
        });
    }

    #[test]
    fn brush_needs_model() {
        with_rig(1, None, |actors, mut physics, _| {
            assert!(matches!(
                physics.set_solid(&actors[0], SolidKind::Brush, None, PhysicsFamily::Q2, None),
                Err(PhysicsError::BrushNeedsModel)
            ));
            physics
                .set_solid(&actors[0], SolidKind::Brush, Some(3), PhysicsFamily::Q2, None)
                .expect("solid");
            assert!(physics.is_brush(actors[0].id()));
        });
    }

    #[test]
    fn bind_and_release() {
        struct Still;
        impl SourceBodyPhysics for Still {
            fn collision(&self) -> Option<SharedSolid> {
                None
            }
            fn motion(&self) -> Option<Q2Motion> {
                None
            }
            fn flags(&self) -> SharedPhysicsFlags {
                SharedPhysicsFlags::default()
            }
            fn write_flags(&mut self, _changes: &SharedPhysicsFlags) {}
            fn write_angular_velocity(&mut self, _velocity: Vec3) {}
            fn water_transition(&mut self) {}
        }
        with_rig(1, None, |actors, mut physics, _| {
            let actor = &actors[0];
            physics.bind_source(actor, Box::new(Still)).expect("bind");
            assert!(matches!(
                physics.bind_source(actor, Box::new(Still)),
                Err(PhysicsError::ActorHasSource)
            ));
            physics.release_actor(actor.id());
            assert!(physics.solid_of(actor.id()).is_none());
        });
    }

    #[test]
    fn touch_triggers_q2_dispatches() {
        with_rig(2, None, |actors, mut physics, handles| {
            let (mover, trigger) = (&actors[0], &actors[1]);
            handles.states.borrow_mut().insert(mover.id().clone(), test_body());
            handles.states.borrow_mut().insert(trigger.id().clone(), test_body());
            physics
                .set_solid(mover, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics
                .set_solid(trigger, SolidKind::Trigger, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.link_body(mover);
            physics.link_body(trigger);
            physics.touch_triggers(mover);
            let touches = handles.touches.borrow();
            assert_eq!(touches.len(), 1);
            assert_eq!(touches[0].mover.id(), trigger.id());
            assert_eq!(touches[0].other, *mover.id());
        });
    }

    #[test]
    fn step_toss_lands_on_floor() {
        with_rig(2, Some(1), |actors, mut physics, handles| {
            let (actor, world) = (&actors[0], &actors[1]);
            let mut body = test_body();
            body.velocity = vec3(0.0, 0.0, -100.0);
            handles.states.borrow_mut().insert(actor.id().clone(), body);
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            let mut motion = test_motion(actor, Q2MotionKind::Toss);
            motion.velocity = vec3(0.0, 0.0, -100.0);
            physics.set_motion(motion);
            handles
                .traces
                .borrow_mut()
                .push_back(floor_q2_trace(vec3(0.0, 0.0, 5.0)));
            assert_eq!(physics.step(actor, 0.1).expect("step"), None);
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert_eq!(body.velocity, vec3(0.0, 0.0, 0.0));
            assert_eq!(body.ground, Some(world.id().clone()));
            assert_eq!(handles.touches.borrow().len(), 2);
        });
    }

    #[test]
    fn step_new_toss_moves() {
        with_rig(1, None, |actors, mut physics, handles| {
            let actor = &actors[0];
            handles.states.borrow_mut().insert(actor.id().clone(), test_body());
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.set_motion(test_motion(actor, Q2MotionKind::NewToss));
            assert_eq!(physics.step(actor, 0.1).expect("step"), Some(NewTossOutcome::Moved));
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert!(body.velocity.z < 0.0);
        });
    }

    #[test]
    fn walk_move_advances() {
        with_rig(2, Some(1), |actors, mut physics, handles| {
            let (actor, world) = (&actors[0], &actors[1]);
            handles.states.borrow_mut().insert(actor.id().clone(), test_body());
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.set_motion(test_motion(actor, Q2MotionKind::Step));
            let mut traces = handles.traces.borrow_mut();
            traces.push_back(floor_q2_trace(vec3(5.0, 0.0, 10.0)));
            for _ in 0..5 {
                traces.push_back(floor_q2_trace(vec3(0.0, 0.0, 10.0)));
            }
            drop(traces);
            assert!(physics.walk_move(actor, 0.0, 5.0));
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert!((f64::from(body.origin.x) - 5.0).abs() < 0.01);
            assert_eq!(body.ground, Some(world.id().clone()));
        });
    }

    #[test]
    fn capture_restore_round_trip() {
        with_rig(1, None, |actors, mut physics, handles| {
            let actor = &actors[0];
            handles.states.borrow_mut().insert(actor.id().clone(), test_body());
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.set_motion(test_motion(actor, Q2MotionKind::Toss));
            physics.set_flags(
                actor,
                SharedPhysicsFlags {
                    fly: Some(true),
                    water_level: Some(2),
                    ..SharedPhysicsFlags::default()
                },
            );
            physics.set_collision(
                actor,
                ActorCollision {
                    family: CollisionFamily::Q3,
                    shape: CollisionShape::Model(7),
                    contents: 1,
                    owner: None,
                    role: CollisionRole::Solid,
                    monster: true,
                    dead_monster: false,
                    q1_corpse: false,
                    q3_owner: Some(Q3OwnerRef {
                        entity_number: 4,
                        owner_number: 9,
                    }),
                },
            );
            physics.link_body(actor);
            let json = physics.capture().expect("capture");
            let again = physics.capture().expect("capture");
            assert_eq!(format!("{json:?}"), format!("{again:?}"));
            physics.set_world_gravity(1.0).expect("gravity");
            let reader = SaveReader::new(&json);
            physics.restore_checkpoint(&reader, false, &|_| false).expect("restore");
            assert_eq!(physics.gravity(), 800.0);
            assert_eq!(physics.motion_of(actor.id()).expect("motion").kind, Q2MotionKind::Toss);
            assert_eq!(physics.solid_of(actor.id()).expect("solid").solid, SolidKind::Brush);
            let restored = physics.capture().expect("capture");
            let reader = SaveReader::new(&restored);
            physics.restore_spatial(&reader, &|_| false).expect("spatial");
            let collision = physics.collisions.get(actor.id()).expect("collision");
            assert_eq!(
                collision.q3_owner,
                Some(Q3OwnerRef {
                    entity_number: 4,
                    owner_number: 9,
                })
            );
        });
    }

    #[test]
    fn water_transition_emits() {
        let mut actors = FakeActors::new();
        let actor = actors.mint();
        let bodies = FakeBodies::new();
        let states = bodies.states.clone();
        let mut scene = FakeScene::new();
        scene.contents = PointContents::Q2 { stored: 32, merged: 32 };
        let callbacks = FakeCallbacks::new();
        let mut physics = SharedPhysics::new(
            test_options(None),
            Box::new(actors),
            Box::new(bodies),
            Box::new(scene),
            Box::new(callbacks),
        )
        .expect("physics");
        states.borrow_mut().insert(actor.id().clone(), test_body());
        physics
            .set_solid(&actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
            .expect("solid");
        let before = vec3(0.0, 0.0, 20.0);
        physics.water_transition(&actor, before);
        let events = physics.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, PhysicsEventKind::WaterEnter);
        assert_eq!(events[0].origin, before);
    }

    #[test]
    fn q1_pusher_snapshot_round_trip() {
        with_rig(2, Some(1), |actors, mut physics, handles| {
            let (actor, world) = (&actors[0], &actors[1]);
            let mut body = test_body();
            body.ground = Some(world.id().clone());
            handles.states.borrow_mut().insert(actor.id().clone(), body);
            physics
                .set_solid(actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
                .expect("solid");
            physics.set_motion(test_motion(actor, Q2MotionKind::Step));
            physics.set_flags(
                actor,
                SharedPhysicsFlags {
                    fly: Some(true),
                    ..SharedPhysicsFlags::default()
                },
            );
            physics.link_body(actor);
            let entity = physics.read_q1_pusher(actor.id()).expect("pusher");
            assert_eq!(entity.state.move_type, 4);
            assert_eq!(entity.state.flags, 513);
            let mut cleared = entity;
            cleared.state.flags = 0;
            physics.write_q1_pusher(&cleared);
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert!(body.ground.is_none());
        });
    }

    #[test]
    fn push_team_moves_pusher() {
        with_rig(1, None, |actors, mut physics, handles| {
            let actor = &actors[0];
            let mut body = test_body();
            body.velocity = vec3(100.0, 0.0, 0.0);
            handles.states.borrow_mut().insert(actor.id().clone(), body);
            physics
                .set_solid(actor, SolidKind::Brush, Some(1), PhysicsFamily::Q2, None)
                .expect("solid");
            let mut motion = test_motion(actor, Q2MotionKind::Push);
            motion.velocity = vec3(100.0, 0.0, 0.0);
            physics.set_motion(motion);
            physics.link_body(actor);
            let blocked = physics.push_team(std::slice::from_ref(actor), 0.1).expect("team");
            assert!(blocked.is_none());
            let body = handles.states.borrow().get(actor.id()).cloned().expect("body");
            assert!((f64::from(body.origin.x) - 10.0).abs() < 0.01);
        });
    }

    #[test]
    fn rerelease_edition_slides() {
        let mut actors = FakeActors::new();
        let actor = actors.mint();
        let bodies = FakeBodies::new();
        let states = bodies.states.clone();
        let scene = FakeScene::new();
        let callbacks = FakeCallbacks::new();
        let mut options = test_options(None);
        options.q2_edition = Some(Q2Edition::Rerelease);
        let mut physics = SharedPhysics::new(
            options,
            Box::new(actors),
            Box::new(bodies),
            Box::new(scene),
            Box::new(callbacks),
        )
        .expect("physics");
        states.borrow_mut().insert(actor.id().clone(), test_body());
        physics
            .set_solid(&actor, SolidKind::Box, None, PhysicsFamily::Q2, None)
            .expect("solid");
        physics.set_motion(test_motion(&actor, Q2MotionKind::Step));
        assert_eq!(physics.step(&actor, 0.1).expect("step"), None);
        let body = states.borrow().get(actor.id()).cloned().expect("body");
        assert!(body.origin.z < 10.0);
    }
}
