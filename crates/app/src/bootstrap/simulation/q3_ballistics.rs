//! Quake III shared weapon ballistics over session actors and scene.
//!
//! Port of donor `src/app/bootstrap/simulation/q3-ballistics.ts`
//! (`Q3SharedBallistics`, `readQ3ProjectileStates`,
//! `readQ3WeaponStatistics`). The host traits below mirror
//! `Q3SharedBallisticsHost`: actor, scene, combat, random-access, and
//! weapon-behavior services come from the session owner, while hitscan,
//! projectile, radius-damage, and grapple math reuse
//! `qa-content`. The trace policy is baked into
//! [`Q3BallisticsScene`] (the donor passes an explicit Q3 policy and
//! rejects any other result kind, which the Q3-only result type makes
//! unnecessary here). Random draws bridge through a
//! [`GameRandomMirror`] seeded from the host stream, syncing the seed
//! back after each call so the donor's single stream stays exact.
//! `WeaponBehaviorProjectilePort` (donor `contracts/weapon-behavior.ts`) is the canonical
//! [`WeaponBehaviorProjectilePort`](qa_content::q2::support::contracts::WeaponBehaviorProjectilePort)
//! port. [`Q3WeaponBehaviorPort`] keeps only the launch/step overrides this file consumes,
//! and [`CanonicalWeaponBehaviorPort`] adapts any canonical port onto it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::ProjectileRole;
use qa_content::q2::support::contracts::{
    BodyState as ContractBodyState, WeaponBehaviorLaunch, WeaponBehaviorProjectilePort, WeaponTrajectoryUpdate,
};
use qa_content::q3::base::game::ballistics_math::{q3_missile_parameters, q3_nail_velocity};
use qa_content::q3::base::game::entities::{GameRandomMirror, SimRandom};
use qa_content::q3::base::game::grapple::{
    q3_grapple_cable, q3_grapple_target, q3_grapple_velocity, Q3_GRAPPLE_LIFETIME, Q3_GRAPPLE_SPEED,
    Q3_GRAPPLE_THINK_INTERVAL,
};
use qa_content::q3::base::game::hitscan::{
    q3_accuracy_hit, q3_bullet_fire, q3_gauntlet_attack, q3_lightning_fire, q3_rail_fire, q3_rail_statistics,
    q3_shotgun_fire, BulletEmitEvent, HitscanProduct, Q3AccuracySubject, Q3BulletAttack, Q3BulletHost, Q3BulletTarget,
    Q3ContactEvent, Q3ContactHost, Q3RailHost, Q3RailStatistics, Q3RailTrail, Q3ShotgunEvent, Q3ShotgunHost,
    RailRestore,
};
use qa_content::q3::base::game::missile::{snap_vector, snap_vector_towards};
use qa_content::q3::base::game::projectile::{
    q3_explode_projectile, q3_launch_projectile, q3_step_projectile, ProjectilePhase, Q3Projectile, Q3ProjectileHost,
    Q3ProjectileImpact, Q3ProjectileTarget,
};
use qa_content::q3::base::game::radius_damage::{q3_radius_damage, Q3RadiusHost, Q3RadiusTarget};
use qa_content::q3::base::game::state::SpatialQueries;
use qa_content::q3::base::records::{AttackProvenance, DamageRequest, Q3SessionBodies, Q3SessionCombat};
use qa_content::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use qa_content::q3::base::world::{ActorTraceHit, ActorTraceQuery, ActorTraceResult, TraceContact, TraceShape};
use qa_content::q3::foundation::arsenal::WeaponStepInput;
use qa_content::q3::foundation::weapon_behavior::q3_projectile_behavior;
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{add3, angle_vectors, normalize3, scale3, vec3, Bounds, Vec3};
use qa_core::numeric::{qvm_float_to_int, NumericProfile};
use qa_world::body::BodyState;
use qa_world::combat::Delivery;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{arr, boolean, int, obj, str as json_str, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

/// Shared ballistics failure (donor `Error` throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3BallisticsError {
    /// Unsupported weapon index in `fire`.
    #[error("Q3 shared ballistics does not yet support weapon {0}")]
    UnsupportedWeapon(i32),
    /// Saved player carries two grappling hooks.
    #[error("Saved Q3 player has multiple grappling hooks")]
    DuplicateHook,
    /// Launched projectile lost its authoritative body.
    #[error("Launched Q3 projectile lost its body")]
    LaunchBodyLost,
    /// Projectile lost its authoritative body mid-step.
    #[error("Q3 projectile lost its authoritative body")]
    StepBodyLost,
    /// Attached hook lost its body.
    #[error("Attached Q3 hook lost its body")]
    HookBodyLost,
    /// Retained projectile has no source impact event.
    #[error("Retained Q3 projectile has no source impact event")]
    ImpactMissing,
    /// Actor handle is not owned by the session.
    #[error("Q3 ballistic actor is not owned: {0}")]
    ActorNotOwned(String),
    /// Weapon behavior lookup failed.
    #[error("{0}")]
    Behavior(String),
}

/// Shooter pose for one ballistic frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BallisticPose {
    /// Shooter origin.
    pub origin: Vec3,
    /// Shooter angles.
    pub angles: Vec3,
    /// View height above the origin.
    pub viewheight: f32,
    /// Quad damage multiplier.
    pub quad: f32,
    /// Whether quad damage is active.
    pub quad_active: bool,
}

/// Impact surface for an `impact` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ImpactKind {
    /// World surface.
    Wall,
    /// Actor flesh.
    Flesh,
}

/// Kind-specific payload of a shared ballistic event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3BallisticEventKind {
    /// Actor removed.
    Remove,
    /// Projectile bounce.
    Bounce,
    /// Trail segment.
    Trail,
    /// Weapon fired.
    Fire {
        /// Muzzle volume.
        volume: f32,
    },
    /// Projectile state moved.
    Projectile {
        /// Current trajectory.
        trajectory: Trajectory,
    },
    /// Projectile impact.
    Impact {
        /// Impact surface.
        hit_kind: Q3ImpactKind,
    },
    /// Contact-weapon event.
    Contact {
        /// Contact event.
        contact: Q3ContactEvent,
    },
    /// Shotgun pellet event.
    Shotgun {
        /// Pellet event.
        shot: Q3ShotgunEvent,
    },
    /// Rail trail.
    Rail {
        /// Trail event.
        trail: Q3RailTrail,
    },
    /// Rail impressive award.
    RailAward {
        /// Impressive count.
        count: i32,
        /// Reward expiry time.
        until: i32,
    },
}

/// Shared ballistic event with its frame timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SharedBallisticEvent {
    /// Acting actor.
    pub actor: ActorId,
    /// Weapon index.
    pub weapon: i32,
    /// Event origin.
    pub origin: Vec3,
    /// Event end.
    pub end: Vec3,
    /// Event normal.
    pub normal: Vec3,
    /// Target actor, if any.
    pub target: Option<ActorId>,
    /// Surface flags.
    pub surface_flags: i32,
    /// Kind-specific payload.
    pub kind: Q3BallisticEventKind,
    /// Frame time in milliseconds.
    pub time_milliseconds: i32,
}

/// Per-actor rail accuracy counters plus the shot count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3WeaponStatistics {
    /// Counted actor.
    pub actor: OwnedActor,
    /// Shots fired.
    pub shots: i32,
    /// Accuracy hits.
    pub hits: i32,
    /// Consecutive multi-hit streak.
    pub streak: i32,
    /// Impressive award count.
    pub impressive_count: i32,
    /// Reward expiry time.
    pub reward_until: i32,
}

impl Q3WeaponStatistics {
    /// Zero counters for an actor.
    #[must_use]
    pub fn zero(actor: OwnedActor) -> Self {
        Self {
            actor,
            shots: 0,
            hits: 0,
            streak: 0,
            impressive_count: 0,
            reward_until: 0,
        }
    }

    /// Rail statistics word.
    #[must_use]
    pub fn rail(&self) -> Q3RailStatistics {
        Q3RailStatistics {
            streak: self.streak,
            hits: self.hits,
            impressive_count: self.impressive_count,
            reward_until: self.reward_until,
        }
    }
}

/// Retained projectile impact record (the `impact` impact kind).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ImpactRecord {
    /// Impact normal.
    pub normal: Vec3,
    /// Target actor, if any.
    pub target: Option<ActorId>,
    /// Whether the impact hit flesh.
    pub flesh: bool,
    /// Surface flags.
    pub surface_flags: i32,
}

impl From<&Q3ImpactRecord> for Q3ProjectileImpact {
    fn from(record: &Q3ImpactRecord) -> Self {
        Q3ProjectileImpact::Impact {
            normal: record.normal,
            target: record.target.clone(),
            flesh: record.flesh,
            surface_flags: record.surface_flags,
        }
    }
}

/// Projectile phase word.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ProjectilePhase {
    /// In flight.
    Flight,
    /// Retained impact event.
    Event {
        /// Event time in milliseconds.
        time: i32,
        /// Retained impact.
        impact: Q3ImpactRecord,
    },
    /// Grappling hook attached.
    Attached {
        /// Attached player target, if any.
        target: Option<ActorId>,
        /// Next think time in milliseconds.
        next_think: i32,
    },
}

/// Live projectile state: the shared base plus expiry and phase.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ProjectileState {
    /// Shared projectile base.
    pub base: Q3Projectile,
    /// Expiry time in milliseconds.
    pub expires: i32,
    /// Phase word.
    pub phase: Q3ProjectilePhase,
}

/// Save checkpoint phase word with save-safe actor references.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ProjectileCheckpointPhase {
    /// In flight.
    Flight,
    /// Retained impact event.
    Event {
        /// Event time in milliseconds.
        time: i32,
        /// Retained impact normal.
        normal: Vec3,
        /// Retained impact target.
        target: Option<SavedActorId>,
        /// Whether the impact hit flesh.
        flesh: bool,
        /// Surface flags.
        surface_flags: i32,
    },
    /// Grappling hook attached.
    Attached {
        /// Attached player target, if any.
        target: Option<SavedActorId>,
        /// Next think time in milliseconds.
        next_think: i32,
    },
}

/// Save checkpoint for one projectile.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ProjectileCheckpoint {
    /// Projectile actor.
    pub actor: SavedActorId,
    /// Owner actor.
    pub owner: SavedActorId,
    /// Passed-through actor, if any.
    pub pass: Option<SavedActorId>,
    /// Damage point.
    pub damage_point: Vec3,
    /// Projectile flags.
    pub flags: i32,
    /// Weapon index.
    pub weapon: i32,
    /// Direct damage.
    pub direct: i32,
    /// Splash damage.
    pub splash: i32,
    /// Splash radius.
    pub radius: i32,
    /// Means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
    /// Expiry time in milliseconds.
    pub expires: i32,
    /// Current trajectory.
    pub trajectory: Trajectory,
    /// Phase word.
    pub phase: Q3ProjectileCheckpointPhase,
}

/// Session actor services consumed by shared ballistics.
pub trait Q3BallisticsActors {
    /// Allocate a projectile actor.
    fn allocate(&self, provider: &ProviderId, definition: &str) -> OwnedActor;
    /// Reject handles the session does not own.
    fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BallisticsError>;
    /// Resolve a live owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Release an actor.
    fn release(&self, actor: &OwnedActor);
    /// Observe actor release; returns an unobserve callback.
    fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()>;
}

/// Shared scene queries consumed by shared ballistics.
pub trait Q3BallisticsScene {
    /// Trace with the source Q3 collision policy.
    fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        pass_actor: Option<&ActorId>,
        shape: TraceShape,
        mask: i32,
        numeric: NumericProfile,
    ) -> ActorTraceResult;
    /// Actors overlapping bounds, at most `maximum`.
    fn query_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId>;
}

/// Weapon behavior launch request (minimal `WeaponBehaviorProjectilePort`
/// launch word).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BehaviorLaunch {
    /// Projectile actor.
    pub projectile: OwnedActor,
    /// Shooter actor.
    pub shooter: ActorId,
    /// Weapon item.
    pub weapon: String,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Weapon behavior overrides consumed by shared ballistics (minimal
/// [`WeaponBehaviorProjectilePort`] view).
pub trait Q3WeaponBehaviorPort {
    /// Override a launch body, if the behavior replaces it.
    fn launch(&self, request: &Q3BehaviorLaunch) -> Option<BodyState>;
    /// Override a flight body, if the behavior replaces it.
    fn step(&self, projectile: &OwnedActor, body: &BodyState, time_seconds: f64) -> Option<BodyState>;
}

/// Canonical-port adapter: any canonical [`WeaponBehaviorProjectilePort`] plugs into
/// shared ballistics through this wrapper, which merges trajectory updates into bodies.
pub struct CanonicalWeaponBehaviorPort<P>(pub RefCell<P>);

impl<P: WeaponBehaviorProjectilePort> Q3WeaponBehaviorPort for CanonicalWeaponBehaviorPort<P> {
    fn launch(&self, request: &Q3BehaviorLaunch) -> Option<BodyState> {
        let update = self.0.borrow_mut().launch(&WeaponBehaviorLaunch {
            projectile: request.projectile.clone(),
            shooter: request.shooter.clone(),
            weapon: request.weapon.clone(),
            role: request.role,
            time_seconds: request.time_seconds,
            body: contract_body(&request.body),
        })?;
        Some(apply_trajectory_update(&request.body, &update))
    }

    fn step(&self, projectile: &OwnedActor, body: &BodyState, time_seconds: f64) -> Option<BodyState> {
        let update = self
            .0
            .borrow_mut()
            .step(projectile, &contract_body(body), time_seconds)?;
        Some(apply_trajectory_update(body, &update))
    }
}

/// Project a session body onto the canonical contract body shape.
fn contract_body(body: &BodyState) -> ContractBodyState {
    ContractBodyState {
        origin: body.origin,
        angles: body.angles,
        velocity: body.velocity,
        bounds: body.bounds,
        ground: body.ground.clone(),
    }
}

/// Merge a canonical trajectory update into its launch/flight body.
fn apply_trajectory_update(body: &BodyState, update: &WeaponTrajectoryUpdate) -> BodyState {
    BodyState {
        origin: update.origin,
        velocity: update.velocity,
        angles: update.angles,
        ..body.clone()
    }
}

/// Registered projectile step callback.
pub type ProjectileStep = Rc<dyn Fn(i32, i32) -> Result<(), Q3BallisticsError>>;

/// Session services behind [`Q3SharedBallistics`].
pub trait Q3SharedBallisticsHost {
    /// Session actors.
    fn actors(&self) -> Rc<dyn Q3BallisticsActors>;
    /// Shared bodies.
    fn bodies(&self) -> Rc<dyn Q3SessionBodies>;
    /// Shared scene.
    fn scene(&self) -> Rc<dyn Q3BallisticsScene>;
    /// Gameplay authority.
    fn combat(&self) -> Rc<dyn Q3SessionCombat>;
    /// Weapon behavior overrides, if any.
    fn weapon_behavior(&self) -> Option<Rc<dyn Q3WeaponBehaviorPort>>;
    /// Projectile provider.
    fn weapon_provider(&self) -> ProviderId;
    /// Numeric profile.
    fn numeric(&self) -> NumericProfile;
    /// Shooter pose.
    fn pose(&self, actor: &OwnedActor) -> Q3BallisticPose;
    /// Frame time in milliseconds.
    fn time(&self) -> i32;
    /// Whether team deathmatch scoring applies.
    fn team_deathmatch(&self) -> bool;
    /// Whether team play applies.
    fn team_game(&self) -> bool;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// World actor.
    fn world_actor(&self) -> ActorId;
    /// Muzzle volume multiplier.
    fn weapon_volume(&self, _actor: &ActorId) -> f32 {
        1.0
    }
    /// Record a weapon impact mark.
    fn weapon_impact(&self, _owner: &ActorId, _point: Vec3) {}
    /// Build attack provenance for a damage request.
    fn attack(
        &self,
        actor: &ActorId,
        inflictor: &ActorId,
        weapon: i32,
        method: i32,
        flags: i32,
        originating_projectile: Option<&ActorId>,
    ) -> AttackProvenance;
    /// Publish a ballistic event.
    fn event(&self, event: Q3SharedBallisticEvent);
    /// Register a projectile step callback.
    fn track_projectile(&self, actor: &OwnedActor, owner: &ActorId, step: ProjectileStep);
    /// Current random stream seed.
    fn random_seed(&self) -> i32;
    /// Restore the random stream seed.
    fn set_random_seed(&self, seed: i32);
}

/// Q3 weapon trajectories and damage over the session's existing actors
/// and collision scene.
#[derive(Clone)]
pub struct Q3SharedBallistics {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    host: Rc<dyn Q3SharedBallisticsHost>,
    projectiles: HashMap<ActorId, Q3ProjectileState>,
    weapon_counters: HashMap<ActorId, Q3WeaponStatistics>,
    hooks: HashMap<ActorId, OwnedActor>,
    hook_held: HashSet<ActorId>,
    deferred_explode: Option<ActorId>,
}

fn zero() -> Vec3 {
    vec3(0.0, 0.0, 0.0)
}

fn contact_normal(trace: &ActorTraceResult) -> Vec3 {
    match &trace.contact {
        TraceContact::Plane { plane } => plane.normal,
        TraceContact::None => zero(),
    }
}

fn hit_actor(trace: &ActorTraceResult) -> Option<ActorId> {
    match &trace.hit {
        ActorTraceHit::Actor { actor } => Some(actor.clone()),
        ActorTraceHit::None | ActorTraceHit::World => None,
    }
}

impl Q3SharedBallistics {
    /// Bind ballistics to a session host.
    #[must_use]
    pub fn new(host: Rc<dyn Q3SharedBallisticsHost>) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            host,
            projectiles: HashMap::new(),
            weapon_counters: HashMap::new(),
            hooks: HashMap::new(),
            hook_held: HashSet::new(),
            deferred_explode: None,
        }));
        let weak = Rc::downgrade(&inner);
        let actors = inner.borrow().host.actors();
        let _unobserve = actors.on_release(Box::new(move |actor| {
            if let Some(inner) = weak.upgrade() {
                Self::released(&inner, actor);
            }
        }));
        Self { inner }
    }

    fn host(&self) -> Rc<dyn Q3SharedBallisticsHost> {
        self.inner.borrow().host.clone()
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        actor: ActorId,
        weapon: i32,
        origin: Vec3,
        end: Vec3,
        normal: Vec3,
        target: Option<ActorId>,
        surface_flags: i32,
        kind: Q3BallisticEventKind,
    ) {
        let host = self.host();
        host.event(Q3SharedBallisticEvent {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            kind,
            time_milliseconds: host.time(),
        });
    }

    fn released(inner: &Rc<RefCell<Inner>>, actor: &OwnedActor) {
        let id = actor.id().clone();
        let host = inner.borrow().host.clone();
        let hook = {
            let mut guard = inner.borrow_mut();
            guard.weapon_counters.remove(&id);
            guard.hook_held.remove(&id);
            guard.hooks.remove(&id)
        };
        // Donor `onRelease` runs `releaseHook` before projectile
        // cleanup; releasing the hook actor reenters here for the hook.
        if let Some(hook) = hook {
            if host.actors().is_live(hook.id()) {
                host.actors().release(&hook);
            }
        }
        let removed = inner.borrow_mut().projectiles.remove(&id);
        let Some(projectile) = removed else {
            return;
        };
        {
            let mut guard = inner.borrow_mut();
            let owned_hook = guard.hooks.get(&projectile.base.owner).cloned();
            if owned_hook.is_some_and(|hook| hook.id() == &id) {
                guard.hooks.remove(&projectile.base.owner);
            }
        }
        let this = Self {
            inner: Rc::clone(inner),
        };
        let base = projectile.base.trajectory.base;
        this.emit(
            id,
            projectile.base.weapon,
            base,
            base,
            zero(),
            None,
            0,
            Q3BallisticEventKind::Remove,
        );
    }

    /// Checkpoint live projectiles with save-safe actor references.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<Q3ProjectileCheckpoint> {
        self.inner
            .borrow()
            .projectiles
            .values()
            .map(|state| {
                let base = &state.base;
                Q3ProjectileCheckpoint {
                    actor: SavedActorId::from(&base.actor),
                    owner: SavedActorId::from(&base.owner),
                    pass: base.pass.as_ref().map(SavedActorId::from),
                    damage_point: base.damage_point,
                    flags: base.flags,
                    weapon: base.weapon,
                    direct: base.direct,
                    splash: base.splash,
                    radius: base.radius,
                    method: base.method,
                    splash_method: base.splash_method,
                    expires: state.expires,
                    trajectory: base.trajectory,
                    phase: match &state.phase {
                        Q3ProjectilePhase::Flight => Q3ProjectileCheckpointPhase::Flight,
                        Q3ProjectilePhase::Attached { target, next_think } => Q3ProjectileCheckpointPhase::Attached {
                            target: target.as_ref().map(SavedActorId::from),
                            next_think: *next_think,
                        },
                        Q3ProjectilePhase::Event { time, impact } => Q3ProjectileCheckpointPhase::Event {
                            time: *time,
                            normal: impact.normal,
                            target: impact.target.as_ref().map(SavedActorId::from),
                            flesh: impact.flesh,
                            surface_flags: impact.surface_flags,
                        },
                    },
                }
            })
            .collect()
    }

    /// Checkpoint grapple-hold actors.
    #[must_use]
    pub fn checkpoint_hook_held(&self) -> Vec<SavedActorId> {
        self.inner.borrow().hook_held.iter().map(SavedActorId::from).collect()
    }

    /// Restore grapple-hold actors.
    pub fn restore_hook_held(&self, actors: &[OwnedActor]) -> Result<(), Q3BallisticsError> {
        let host = self.host();
        for actor in actors {
            host.actors().assert_owned(actor)?;
        }
        let mut guard = self.inner.borrow_mut();
        guard.hook_held.clear();
        for actor in actors {
            guard.hook_held.insert(actor.id().clone());
        }
        Ok(())
    }

    /// Restore checkpointed projectiles and re-register their steps.
    pub fn restore(&self, states: Vec<Q3ProjectileState>) -> Result<(), Q3BallisticsError> {
        let host = self.host();
        let mut resolved = Vec::with_capacity(states.len());
        for state in states {
            let owned = host.actors().resolve_owned(&state.base.actor).ok_or_else(|| {
                Q3BallisticsError::ActorNotOwned("missing owned handle for projectile restore".to_string())
            })?;
            host.actors().assert_owned(&owned)?;
            resolved.push((owned, state));
        }
        {
            let mut guard = self.inner.borrow_mut();
            guard.projectiles.clear();
            guard.hooks.clear();
            for (owned, state) in resolved {
                if state.base.weapon == 10 {
                    if guard.hooks.contains_key(&state.base.owner) {
                        return Err(Q3BallisticsError::DuplicateHook);
                    }
                    guard.hooks.insert(state.base.owner.clone(), owned);
                }
                guard.projectiles.insert(state.base.actor.clone(), state);
            }
        }
        let keys: Vec<ActorId> = self.inner.borrow().projectiles.keys().cloned().collect();
        for key in keys {
            let (owner, phase) = {
                let guard = self.inner.borrow();
                let state = guard
                    .projectiles
                    .get(&key)
                    .unwrap_or_else(|| panic!("restored Q3 projectile vanished during registration"));
                (state.base.owner.clone(), state.phase.clone())
            };
            let owned = host.actors().resolve_owned(&key).ok_or_else(|| {
                Q3BallisticsError::ActorNotOwned("missing owned handle for projectile restore".to_string())
            })?;
            self.track(&owned, &owner);
            if let Q3ProjectilePhase::Event { time, impact } = phase {
                self.impact_event_at(&key, &impact, time);
            }
        }
        Ok(())
    }

    fn track(&self, actor: &OwnedActor, owner: &ActorId) {
        let weak = Rc::downgrade(&self.inner);
        let key = actor.id().clone();
        let step: ProjectileStep = Rc::new(move |previous, time| {
            if let Some(inner) = weak.upgrade() {
                Self::step_actor(&inner, &key, previous, time)
            } else {
                Ok(())
            }
        });
        self.host().track_projectile(actor, owner, step);
    }

    /// Accuracy counters for an actor, defaulting to zero.
    #[must_use]
    pub fn weapon_statistics(&self, actor: &OwnedActor) -> Q3WeaponStatistics {
        self.inner
            .borrow()
            .weapon_counters
            .get(actor.id())
            .cloned()
            .unwrap_or_else(|| Q3WeaponStatistics::zero(actor.clone()))
    }

    /// Checkpoint accuracy counters.
    #[must_use]
    pub fn checkpoint_weapon_statistics(&self) -> Vec<Q3WeaponStatistics> {
        self.inner.borrow().weapon_counters.values().cloned().collect()
    }

    /// Restore accuracy counters.
    pub fn restore_weapon_statistics(&self, states: Vec<Q3WeaponStatistics>) -> Result<(), Q3BallisticsError> {
        let host = self.host();
        for state in &states {
            host.actors().assert_owned(&state.actor)?;
        }
        let mut guard = self.inner.borrow_mut();
        guard.weapon_counters.clear();
        for state in states {
            guard.weapon_counters.insert(state.actor.id().clone(), state);
        }
        Ok(())
    }

    /// Reset hook and streak state for a respawning actor.
    pub fn respawn(&self, actor: &ActorId) {
        let host = self.host();
        let owner = host.actors().resolve_owned(actor);
        self.release_hook(actor);
        let mut guard = self.inner.borrow_mut();
        if let Some(owner) = owner {
            guard.hook_held.remove(owner.id());
            if let Some(state) = guard.weapon_counters.get(owner.id()).cloned() {
                guard.weapon_counters.insert(
                    owner.id().clone(),
                    Q3WeaponStatistics {
                        streak: 0,
                        reward_until: 0,
                        ..state
                    },
                );
            }
        }
    }

    /// Release the grappling hook owned by an actor, if any.
    pub fn release_hook(&self, actor: &ActorId) {
        let hook = self.inner.borrow_mut().hooks.remove(actor);
        if let Some(hook) = hook {
            if self.host().actors().is_live(hook.id()) {
                self.host().actors().release(&hook);
            }
        }
    }

    /// Track grapple-hold input and release stale hooks.
    pub fn command(&self, actor: &OwnedActor, attack: bool, selected: bool, alive: bool) {
        if !attack || !alive {
            self.inner.borrow_mut().hook_held.remove(actor.id());
        }
        if !attack || !selected || !alive {
            self.release_hook(actor.id());
        }
    }

    /// Attached grapple point for an actor, if any.
    #[must_use]
    pub fn grapple_point(&self, actor: &ActorId) -> Option<Vec3> {
        let guard = self.inner.borrow();
        let hook = guard.hooks.get(actor)?;
        let state = guard.projectiles.get(hook.id())?;
        if !matches!(state.phase, Q3ProjectilePhase::Attached { .. }) {
            return None;
        }
        let bodies = guard.host.bodies();
        let id = hook.id().clone();
        drop(guard);
        bodies.read(&id).map(|body| body.origin)
    }

    /// Grapple pull velocity for an actor, if attached.
    #[must_use]
    pub fn pull(&self, actor: &OwnedActor) -> Option<Vec3> {
        let point = self.grapple_point(actor.id())?;
        let host = self.host();
        let body = host.bodies().read(actor.id())?;
        Some(q3_grapple_velocity(
            body.origin,
            point,
            angle_vectors(host.pose(actor).angles).forward,
        ))
    }

    fn accuracy_subject(&self, actor: &ActorId) -> Q3AccuracySubject {
        let host = self.host();
        let state = host.combat().read(actor);
        Q3AccuracySubject {
            actor: actor.clone(),
            damageable: state.as_ref().is_some_and(|state| state.can_take_damage),
            player: host.is_player(actor),
            health: state.as_ref().map_or(0, |state| state.health),
            team: state.as_ref().and_then(|state| state.team.clone()),
        }
    }

    fn trace_point(&self, start: Vec3, end: Vec3, pass: Option<&ActorId>, mask: i32) -> ActorTraceResult {
        let host = self.host();
        host.scene()
            .trace(start, end, pass, TraceShape::Point, mask, host.numeric())
    }

    #[allow(clippy::too_many_arguments)]
    fn hit(
        &self,
        actor: &ActorId,
        inflictor: &ActorId,
        weapon: i32,
        target: &ActorId,
        point: Vec3,
        direction: Vec3,
        amount: i32,
        method: i32,
        radius: bool,
        originating_projectile: Option<&ActorId>,
    ) {
        let host = self.host();
        if !host.combat().read(target).is_some_and(|state| state.can_take_damage) {
            return;
        }
        host.combat().apply(DamageRequest {
            attack: host.attack(
                actor,
                inflictor,
                weapon,
                method,
                i32::from(radius),
                originating_projectile,
            ),
            target: target.clone(),
            amount: amount as f32,
            knockback: amount as f32,
            direction,
            point,
            normal: zero(),
            delivery: if radius { Delivery::Radius } else { Delivery::Direct },
        });
    }

    fn attack_frame(&self, actor: &OwnedActor) -> Q3BulletAttack {
        let host = self.host();
        let pose = host.pose(actor);
        let vectors = angle_vectors(pose.angles);
        Q3BulletAttack {
            forward: vectors.forward,
            right: vectors.right,
            up: vectors.up,
            muzzle: snap_vector(add3(
                vec3(pose.origin.x, pose.origin.y, pose.origin.z + pose.viewheight),
                scale3(vectors.forward, 14.0),
            )),
            quad: pose.quad,
        }
    }

    /// Credit one accuracy hit for a live actor.
    fn credit_hit(&self, actor: &OwnedActor) {
        if !self.host().actors().is_live(actor.id()) {
            return;
        }
        let current = self.weapon_statistics(actor);
        self.inner.borrow_mut().weapon_counters.insert(
            actor.id().clone(),
            Q3WeaponStatistics {
                hits: current.hits.wrapping_add(1),
                ..current
            },
        );
    }

    fn bullet(&self, actor: &OwnedActor, weapon: i32, attack: &Q3BulletAttack, spread: f32, amount: f32) {
        let shooter = actor.id().clone();
        let this = self.clone();
        let credit = actor.clone();
        let emit_actor = actor.clone();
        let emit_attack = *attack;
        let damage_actor = actor.clone();
        let session = self.host();
        let mirror = Rc::new(RefCell::new(GameRandomMirror::new(session.random_seed())));
        let host = Q3BulletHost {
            random: Rc::clone(&mirror),
            trace: Rc::new(move |start, end, pass| this.trace_point(start, end, pass.as_ref(), 0x6000001)),
            target: {
                let this = self.clone();
                let shooter = shooter.clone();
                Rc::new(move |target| {
                    this.host().combat().read(target)?;
                    let subject = this.accuracy_subject(target);
                    let attacker = this.accuracy_subject(&shooter);
                    Some(Q3BulletTarget {
                        damageable: subject.damageable,
                        player: subject.player,
                        accuracy_eligible: q3_accuracy_hit(this.host().team_game(), &subject, &attacker),
                        invulnerable: false,
                    })
                })
            },
            impact: {
                let this = self.clone();
                let shooter = shooter.clone();
                Some(Rc::new(move |point: Vec3| {
                    this.host().weapon_impact(&shooter, point);
                }) as Rc<dyn Fn(Vec3)>)
            },
            emit: {
                let this = self.clone();
                Rc::new(move |event: BulletEmitEvent| {
                    this.emit(
                        emit_actor.id().clone(),
                        weapon,
                        emit_attack.muzzle,
                        event.point,
                        event.normal,
                        event.target,
                        0,
                        Q3BallisticEventKind::Impact {
                            hit_kind: if event.flesh {
                                Q3ImpactKind::Flesh
                            } else {
                                Q3ImpactKind::Wall
                            },
                        },
                    );
                })
            },
            damage: {
                let this = self.clone();
                Rc::new(move |target: &ActorId, direction: Vec3, point: Vec3, scaled: i32| {
                    this.hit(
                        damage_actor.id(),
                        damage_actor.id(),
                        weapon,
                        target,
                        point,
                        direction,
                        scaled,
                        3,
                        false,
                        None,
                    );
                })
            },
            credit_accuracy_hit: {
                let this = self.clone();
                Rc::new(move || {
                    this.credit_hit(&credit);
                })
            },
            product: HitscanProduct::Baseq3,
        };
        let mut attack = *attack;
        q3_bullet_fire(&host, &shooter, &mut attack, spread, amount);
        session.set_random_seed(mirror.borrow().seed());
    }

    fn contact_host(&self, actor: &OwnedActor, weapon: i32, attack: &Q3BulletAttack, method: i32) -> Q3ContactHost {
        let shooter = actor.clone();
        let shooter_id = shooter.id().clone();
        let trail_actor = actor.clone();
        let emit_actor = actor.clone();
        let emit_attack = *attack;
        let damage_actor = actor.clone();
        let credit = actor.clone();
        Q3ContactHost {
            trace: {
                let this = self.clone();
                Rc::new(move |start, end, pass| {
                    let trace = this.trace_point(start, end, pass.as_ref(), 0x6000001);
                    if weapon == 6 {
                        this.emit(
                            trail_actor.id().clone(),
                            weapon,
                            start,
                            trace.end,
                            contact_normal(&trace),
                            hit_actor(&trace),
                            trace.surface_flags,
                            Q3BallisticEventKind::Trail,
                        );
                    }
                    trace
                })
            },
            target: {
                let this = self.clone();
                Rc::new(move |target| {
                    this.host().combat().read(target)?;
                    let subject = this.accuracy_subject(target);
                    let attacker = this.accuracy_subject(shooter.id());
                    Some(Q3BulletTarget {
                        damageable: subject.damageable,
                        player: subject.player,
                        accuracy_eligible: q3_accuracy_hit(this.host().team_game(), &subject, &attacker),
                        invulnerable: false,
                    })
                })
            },
            impact: {
                let this = self.clone();
                let shooter = shooter_id.clone();
                Some(Rc::new(move |point: Vec3| {
                    this.host().weapon_impact(&shooter, point);
                }) as Rc<dyn Fn(Vec3)>)
            },
            emit: {
                let this = self.clone();
                Rc::new(move |contact: Q3ContactEvent| {
                    let origin = if matches!(contact, Q3ContactEvent::GauntletQuad) {
                        this.host().pose(&emit_actor).origin
                    } else {
                        emit_attack.muzzle
                    };
                    this.emit(
                        emit_actor.id().clone(),
                        weapon,
                        origin,
                        emit_attack.muzzle,
                        zero(),
                        None,
                        0,
                        Q3BallisticEventKind::Contact { contact },
                    );
                })
            },
            damage: {
                let this = self.clone();
                Rc::new(move |target: &ActorId, direction: Vec3, point: Vec3, amount: i32| {
                    this.hit(
                        damage_actor.id(),
                        damage_actor.id(),
                        weapon,
                        target,
                        point,
                        direction,
                        amount,
                        method,
                        false,
                        None,
                    );
                })
            },
            credit_accuracy_hit: {
                let this = self.clone();
                Rc::new(move || {
                    this.credit_hit(&credit);
                })
            },
            product: HitscanProduct::Baseq3,
        }
    }

    /// Gauntlet strike; returns whether the swing connected.
    #[must_use]
    pub fn gauntlet_hit(&self, actor: &OwnedActor) -> bool {
        let attack = self.attack_frame(actor);
        let quad = self.host().pose(actor).quad_active;
        q3_gauntlet_attack(&self.contact_host(actor, 1, &attack, 2), actor.id(), &attack, quad)
    }

    /// Fire a weapon for an actor.
    pub fn fire(&self, actor: &OwnedActor, weapon: i32, _input: &WeaponStepInput) -> Result<(), Q3BallisticsError> {
        if weapon == 10 {
            let mut guard = self.inner.borrow_mut();
            if guard.hook_held.contains(actor.id()) || guard.hooks.contains_key(actor.id()) {
                guard.hook_held.insert(actor.id().clone());
                return Ok(());
            }
            guard.hook_held.insert(actor.id().clone());
        }
        if weapon != 1 && weapon != 10 {
            let previous = self.weapon_statistics(actor);
            let increment = if weapon == 11 { 15 } else { 1 };
            self.inner.borrow_mut().weapon_counters.insert(
                actor.id().clone(),
                Q3WeaponStatistics {
                    shots: previous.shots.wrapping_add(increment),
                    ..previous
                },
            );
        }
        let host = self.host();
        let attack = self.attack_frame(actor);
        self.emit(
            actor.id().clone(),
            weapon,
            attack.muzzle,
            add3(attack.muzzle, attack.forward),
            zero(),
            None,
            0,
            Q3BallisticEventKind::Fire {
                volume: host.weapon_volume(actor.id()),
            },
        );
        match weapon {
            0 | 1 => Ok(()),
            2 => {
                let amount = if host.team_deathmatch() { 5.0 } else { 7.0 };
                self.bullet(actor, weapon, &attack, 200.0, amount);
                Ok(())
            }
            3 => {
                let contact = self.contact_host(actor, weapon, &attack, 1);
                let session = self.host();
                let mirror = Rc::new(RefCell::new(GameRandomMirror::new(session.random_seed())));
                let begin_actor = actor.clone();
                let begin_this = self.clone();
                let begin = Rc::new(move |muzzle: Vec3, direction: Vec3| {
                    let this = begin_this.clone();
                    let emit_actor = begin_actor.clone();
                    Rc::new(move |seed: i32| {
                        let shot = Q3ShotgunEvent {
                            muzzle,
                            direction,
                            seed,
                        };
                        this.emit(
                            emit_actor.id().clone(),
                            weapon,
                            shot.muzzle,
                            shot.direction,
                            zero(),
                            None,
                            0,
                            Q3BallisticEventKind::Shotgun { shot },
                        );
                    }) as Rc<dyn Fn(i32)>
                });
                let alive_actor = actor.clone();
                let alive_this = self.clone();
                let shotgun = Q3ShotgunHost {
                    trace: contact.trace.clone(),
                    target: contact.target.clone(),
                    impact: contact.impact.clone(),
                    damage: contact.damage.clone(),
                    credit_accuracy_hit: contact.credit_accuracy_hit.clone(),
                    random: Rc::clone(&mirror),
                    begin,
                    alive: Rc::new(move || alive_this.host().actors().is_live(alive_actor.id())),
                    product: HitscanProduct::Baseq3,
                };
                q3_shotgun_fire(&shotgun, actor.id(), &attack);
                session.set_random_seed(mirror.borrow().seed());
                Ok(())
            }
            6 => {
                let mut attack = attack;
                q3_lightning_fire(&self.contact_host(actor, weapon, &attack, 11), actor.id(), &mut attack);
                Ok(())
            }
            7 => {
                let contact = self.contact_host(actor, weapon, &attack, 10);
                let alive_actor = actor.clone();
                let alive_this = self.clone();
                let unlink_this = self.clone();
                let trail_actor = actor.clone();
                let trail_this = self.clone();
                let rail = Q3RailHost {
                    trace: contact.trace.clone(),
                    target: contact.target.clone(),
                    impact: contact.impact.clone(),
                    damage: contact.damage.clone(),
                    alive: Rc::new(move || alive_this.host().actors().is_live(alive_actor.id())),
                    unlink: Rc::new(move |target: &ActorId| {
                        let host = unlink_this.host();
                        let owner = host.actors().resolve_owned(target)?;
                        host.bodies().linked(target)?;
                        host.bodies().unlink(&owner);
                        let restore_this = unlink_this.clone();
                        let restore = move || {
                            let host = restore_this.host();
                            if host.actors().is_live(owner.id()) && host.bodies().read(owner.id()).is_some() {
                                host.bodies().link(&owner, None);
                            }
                        };
                        Some(Box::new(restore) as RailRestore)
                    }),
                    trail: Rc::new(move |trail: Q3RailTrail| {
                        trail_this.emit(
                            trail_actor.id().clone(),
                            weapon,
                            trail.start,
                            trail.end,
                            zero(),
                            None,
                            0,
                            Q3BallisticEventKind::Rail { trail },
                        );
                    }),
                    product: HitscanProduct::Baseq3,
                };
                let mut attack = attack;
                let hits = q3_rail_fire(&rail, actor.id(), &mut attack);
                if !self.host().actors().is_live(actor.id()) {
                    return Ok(());
                }
                let previous = self.weapon_statistics(actor);
                let next = q3_rail_statistics(&previous.rail(), hits, self.host().time());
                self.inner.borrow_mut().weapon_counters.insert(
                    actor.id().clone(),
                    Q3WeaponStatistics {
                        shots: previous.shots,
                        hits: next.statistics.hits,
                        streak: next.statistics.streak,
                        impressive_count: next.statistics.impressive_count,
                        reward_until: next.statistics.reward_until,
                        ..previous
                    },
                );
                if next.awarded {
                    let origin = self.host().pose(actor).origin;
                    self.emit(
                        actor.id().clone(),
                        weapon,
                        origin,
                        origin,
                        zero(),
                        None,
                        0,
                        Q3BallisticEventKind::RailAward {
                            count: next.statistics.impressive_count,
                            until: next.statistics.reward_until,
                        },
                    );
                }
                Ok(())
            }
            4 | 5 | 8 | 9 | 10 => {
                self.launch(actor, weapon, &attack)?;
                Ok(())
            }
            11 => {
                for _ in 0..15 {
                    self.launch(actor, weapon, &attack)?;
                }
                Ok(())
            }
            13 => {
                self.bullet(actor, weapon, &attack, 600.0, 7.0);
                Ok(())
            }
            _ => Err(Q3BallisticsError::UnsupportedWeapon(weapon)),
        }
    }

    /// Launch one projectile for an owner.
    fn launch(&self, owner: &OwnedActor, weapon: i32, attack: &Q3BulletAttack) -> Result<(), Q3BallisticsError> {
        struct Spec {
            speed: f32,
            duration: i32,
            gravity: bool,
            direct: f32,
            splash: f32,
            radius: f32,
            method: i32,
            splash_method: i32,
        }
        let spec = if weapon == 10 {
            Spec {
                speed: Q3_GRAPPLE_SPEED,
                duration: Q3_GRAPPLE_LIFETIME,
                gravity: false,
                direct: 0.0,
                splash: 0.0,
                radius: 0.0,
                method: 23,
                splash_method: 0,
            }
        } else if weapon == 11 {
            Spec {
                speed: 0.0,
                duration: 10000,
                gravity: false,
                direct: 20.0,
                splash: 0.0,
                radius: 0.0,
                method: 23,
                splash_method: 0,
            }
        } else {
            let params = q3_missile_parameters(weapon);
            Spec {
                speed: params.speed,
                duration: params.duration as i32,
                gravity: params.gravity,
                direct: params.direct,
                splash: params.splash,
                radius: params.radius,
                method: params.method,
                splash_method: params.splash_method,
            }
        };
        let host = self.host();
        let grenade = weapon == 4;
        let direction = normalize3(if grenade {
            vec3(attack.forward.x, attack.forward.y, attack.forward.z + 0.2)
        } else {
            attack.forward
        });
        let actor = host.actors().allocate(&host.weapon_provider(), "q3:projectile");
        let time = host.time();
        let launch = q3_launch_projectile(attack.muzzle, direction, spec.speed, spec.gravity, spec.duration, time);
        let mut trajectory = launch.trajectory;
        if weapon == 11 {
            let session = self.host();
            let mirror = Rc::new(RefCell::new(GameRandomMirror::new(session.random_seed())));
            let mut guard = mirror.borrow_mut();
            let velocity = q3_nail_velocity(
                attack.muzzle,
                attack.forward,
                attack.right,
                attack.up,
                &mut *guard as &mut dyn SimRandom,
            );
            drop(guard);
            session.set_random_seed(mirror.borrow().seed());
            trajectory = Trajectory {
                time,
                delta: snap_vector(velocity),
                ..trajectory
            };
        }
        host.bodies().create(
            &actor,
            BodyState {
                origin: attack.muzzle,
                angles: zero(),
                velocity: trajectory.delta,
                bounds: Bounds {
                    min: zero(),
                    max: zero(),
                },
                ground: None,
            },
        );
        let mut state = Q3ProjectileState {
            base: Q3Projectile {
                actor: actor.id().clone(),
                owner: owner.id().clone(),
                weapon,
                direct: qvm_float_to_int(spec.direct * attack.quad),
                splash: qvm_float_to_int(spec.splash * attack.quad),
                radius: spec.radius as i32,
                method: spec.method,
                splash_method: spec.splash_method,
                damage_point: zero(),
                trajectory,
                flags: if grenade { 0x20 } else { 0 },
                pass: Some(owner.id().clone()),
            },
            expires: launch.expires,
            phase: Q3ProjectilePhase::Flight,
        };
        let bodies = host.bodies();
        let body = bodies.read(actor.id()).ok_or(Q3BallisticsError::LaunchBodyLost)?;
        if let Some(behavior) = host.weapon_behavior() {
            let declaration =
                q3_projectile_behavior(weapon).map_err(|error| Q3BallisticsError::Behavior(error.to_string()))?;
            let update = behavior.launch(&Q3BehaviorLaunch {
                projectile: actor.clone(),
                shooter: owner.id().clone(),
                weapon: declaration.weapon,
                role: declaration.role,
                time_seconds: f64::from(time) / 1000.0,
                body: body.clone(),
            });
            if let Some(update) = update {
                bodies.write(&actor, update.clone());
                state.base.trajectory = Trajectory {
                    base: update.origin,
                    delta: update.velocity,
                    time,
                    ..state.base.trajectory
                };
            }
        }
        self.inner
            .borrow_mut()
            .projectiles
            .insert(actor.id().clone(), state.clone());
        if weapon == 10 {
            self.inner.borrow_mut().hooks.insert(owner.id().clone(), actor.clone());
        }
        self.track(&actor, owner.id());
        self.emit(
            actor.id().clone(),
            weapon,
            state.base.trajectory.base,
            state.base.trajectory.base,
            zero(),
            None,
            0,
            Q3BallisticEventKind::Projectile {
                trajectory: state.base.trajectory,
            },
        );
        Ok(())
    }

    /// Emit a retained impact event at an explicit timestamp.
    fn impact_event_at(&self, key: &ActorId, impact: &Q3ImpactRecord, time: i32) {
        let host = self.host();
        let Some(body) = host.bodies().read(key) else {
            return;
        };
        host.event(Q3SharedBallisticEvent {
            actor: key.clone(),
            weapon: self
                .inner
                .borrow()
                .projectiles
                .get(key)
                .map_or(0, |state| state.base.weapon),
            origin: body.origin,
            end: body.origin,
            normal: impact.normal,
            target: impact.target.clone(),
            surface_flags: impact.surface_flags,
            kind: Q3BallisticEventKind::Impact {
                hit_kind: if impact.flesh {
                    Q3ImpactKind::Flesh
                } else {
                    Q3ImpactKind::Wall
                },
            },
            time_milliseconds: time,
        });
    }

    fn step_actor(
        inner: &Rc<RefCell<Inner>>,
        key: &ActorId,
        previous_time: i32,
        time: i32,
    ) -> Result<(), Q3BallisticsError> {
        let this = Self {
            inner: Rc::clone(inner),
        };
        let host = this.host();
        let snapshot = inner.borrow().projectiles.get(key).cloned();
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        if snapshot.base.weapon == 10 {
            let owner_gone = !host.actors().is_live(&snapshot.base.owner)
                || host.combat().read(&snapshot.base.owner).map_or(0, |state| state.health) <= 0;
            let target_gone = matches!(&snapshot.phase, Q3ProjectilePhase::Attached { target: Some(target), .. } if !host.actors().is_live(target));
            if owner_gone || target_gone {
                this.release_hook(&snapshot.base.owner);
                return Ok(());
            }
        }
        let flight_override = matches!(snapshot.phase, Q3ProjectilePhase::Flight)
            .then(|| host.weapon_behavior())
            .flatten()
            .and_then(|behavior| host.bodies().read(key).map(|body| (behavior, body)));
        if let Some((behavior, body)) = flight_override {
            let owned = host.actors().resolve_owned(key).ok_or_else(|| {
                Q3BallisticsError::ActorNotOwned("missing owned handle for behavior step".to_string())
            })?;
            if let Some(update) = behavior.step(&owned, &body, f64::from(previous_time) / 1000.0) {
                host.bodies().write(&owned, update.clone());
                if let Some(state) = inner.borrow_mut().projectiles.get_mut(key) {
                    state.base.trajectory = Trajectory {
                        base: update.origin,
                        delta: update.velocity,
                        time: previous_time,
                        ..state.base.trajectory
                    };
                }
            }
        }
        let owned = host
            .actors()
            .resolve_owned(key)
            .ok_or_else(|| Q3BallisticsError::ActorNotOwned("missing owned handle for step".to_string()))?;
        let body = host.bodies().read(key).ok_or(Q3BallisticsError::StepBodyLost)?;
        let base = inner
            .borrow()
            .projectiles
            .get(key)
            .map(|state| state.base.clone())
            .ok_or(Q3BallisticsError::StepBodyLost)?;
        let mut ctx = StepCtx {
            inner: Rc::clone(inner),
            owned,
            key: key.clone(),
            base,
            pending: None,
            step_origin: body.origin,
            previous_time,
            time,
        };
        let mut stepped = ctx.base.clone();
        q3_step_projectile(&mut stepped, &mut ctx);
        ctx.base = stepped;
        if inner.borrow().projectiles.contains_key(key) {
            if let Some(state) = inner.borrow_mut().projectiles.get_mut(key) {
                state.base = ctx.base.clone();
            }
        }
        let deferred = inner.borrow_mut().deferred_explode.take();
        let deferred_base = if deferred.as_ref() == Some(key) {
            inner.borrow().projectiles.get(key).map(|state| state.base.clone())
        } else {
            None
        };
        if let Some(base) = deferred_base {
            let owned = host
                .actors()
                .resolve_owned(key)
                .ok_or_else(|| Q3BallisticsError::ActorNotOwned("missing owned handle for explode".to_string()))?;
            let mut explode = StepCtx {
                inner: Rc::clone(inner),
                owned,
                key: key.clone(),
                base,
                pending: None,
                step_origin: body.origin,
                previous_time,
                time,
            };
            let mut stepped = explode.base.clone();
            q3_explode_projectile(&mut stepped, &mut explode);
            explode.base = stepped;
            if let Some(state) = inner.borrow_mut().projectiles.get_mut(key) {
                state.base = explode.base.clone();
            }
        }
        if snapshot.base.weapon == 10 && host.actors().is_live(key) {
            Self::present_hook(inner, key);
        }
        Ok(())
    }

    fn present_hook(inner: &Rc<RefCell<Inner>>, key: &ActorId) {
        let this = Self {
            inner: Rc::clone(inner),
        };
        let host = this.host();
        let snapshot = inner.borrow().projectiles.get(key).cloned();
        let Some(snapshot) = snapshot else {
            return;
        };
        let Some(owner) = host.actors().resolve_owned(&snapshot.base.owner) else {
            return;
        };
        let Some(body) = host.bodies().read(key) else {
            return;
        };
        if matches!(snapshot.phase, Q3ProjectilePhase::Attached { .. }) {
            this.emit(
                key.clone(),
                10,
                body.origin,
                body.origin,
                zero(),
                None,
                0,
                Q3BallisticEventKind::Projectile {
                    trajectory: snapshot.base.trajectory,
                },
            );
        }
        let pose = host.pose(&owner);
        let cable = q3_grapple_cable(pose.origin, angle_vectors(pose.angles).up, body.origin, pose.viewheight);
        this.emit(
            key.clone(),
            10,
            cable.map_or(body.origin, |cable| cable.start),
            cable.map_or(body.origin, |cable| cable.end),
            zero(),
            Some(snapshot.base.owner.clone()),
            0,
            Q3BallisticEventKind::Trail,
        );
    }

    fn radius(&self, projectile: &Q3Projectile, origin: Vec3, ignore: Option<&ActorId>) -> bool {
        let host = self.host();
        let mut ctx = RadiusCtx {
            inner: Rc::clone(&self.inner),
            spatial: SceneSpatial {
                scene: host.scene(),
                numeric: host.numeric(),
            },
            owner: projectile.owner.clone(),
            weapon: projectile.weapon,
            splash: projectile.splash,
            radius: projectile.radius as f32,
            splash_method: projectile.splash_method,
            projectile: projectile.actor.clone(),
        };
        let splash = ctx.splash as f32;
        let radius = ctx.radius;
        q3_radius_damage(&mut ctx, origin, splash, radius, ignore)
    }
}

/// Per-step projectile host over one cloned base.
struct StepCtx {
    inner: Rc<RefCell<Inner>>,
    owned: OwnedActor,
    key: ActorId,
    base: Q3Projectile,
    pending: Option<Q3ImpactRecord>,
    step_origin: Vec3,
    previous_time: i32,
    time: i32,
}

impl StepCtx {
    fn handle(&self) -> Q3SharedBallistics {
        Q3SharedBallistics {
            inner: Rc::clone(&self.inner),
        }
    }

    fn host(&self) -> Rc<dyn Q3SharedBallisticsHost> {
        self.inner.borrow().host.clone()
    }

    fn body(&mut self) -> BodyState {
        self.host()
            .bodies()
            .read(&self.key)
            .unwrap_or_else(|| panic!("Q3 projectile lost its authoritative body"))
    }

    fn impact_event(&self, impact: &Q3ImpactRecord) {
        let host = self.host();
        let Some(body) = host.bodies().read(&self.key) else {
            return;
        };
        let weapon = self.base.weapon;
        host.event(Q3SharedBallisticEvent {
            actor: self.key.clone(),
            weapon,
            origin: body.origin,
            end: body.origin,
            normal: impact.normal,
            target: impact.target.clone(),
            surface_flags: impact.surface_flags,
            kind: Q3BallisticEventKind::Impact {
                hit_kind: if impact.flesh {
                    Q3ImpactKind::Flesh
                } else {
                    Q3ImpactKind::Wall
                },
            },
            time_milliseconds: self.time,
        });
    }

    fn attach_hook(&mut self, trace: &ActorTraceResult, target: &ActorId) -> bool {
        let host = self.host();
        let target_body = host.bodies().read(target);
        let player = target_body.as_ref().is_some_and(|_| host.is_player(target))
            && host.combat().read(target).is_some_and(|state| state.can_take_damage);
        let point = snap_vector_towards(
            if player {
                let body = target_body.unwrap_or_else(|| panic!("Attached Q3 hook lost its target body"));
                snap_vector_towards(q3_grapple_target(body.origin, &body.bounds), self.base.trajectory.base)
            } else {
                trace.end
            },
            self.base.trajectory.base,
        );
        let body = host
            .bodies()
            .read(&self.key)
            .unwrap_or_else(|| panic!("Attached Q3 hook lost its body"));
        if let Some(state) = self.inner.borrow_mut().projectiles.get_mut(&self.key) {
            state.phase = Q3ProjectilePhase::Attached {
                target: if player { Some(target.clone()) } else { None },
                next_think: self.time.wrapping_add(Q3_GRAPPLE_THINK_INTERVAL),
            };
        }
        self.base.trajectory = Trajectory {
            trajectory_type: TrajectoryType::TrStationary,
            time: 0,
            duration: 0,
            base: point,
            delta: zero(),
        };
        host.bodies().write(
            &self.owned,
            BodyState {
                origin: point,
                velocity: zero(),
                ..body
            },
        );
        host.bodies().link(&self.owned, None);
        self.impact_event(&Q3ImpactRecord {
            normal: contact_normal(trace),
            target: if player { Some(target.clone()) } else { None },
            flesh: player,
            surface_flags: trace.surface_flags,
        });
        true
    }

    fn think_hook(&mut self) {
        let host = self.host();
        let phase = self
            .inner
            .borrow()
            .projectiles
            .get(&self.key)
            .map(|state| state.phase.clone());
        let Some(phase) = phase else {
            return;
        };
        if !matches!(phase, Q3ProjectilePhase::Attached { .. }) {
            if self.time >= self.base_expires() {
                self.handle().release_hook(&self.base.owner.clone());
            }
            return;
        }
        let Q3ProjectilePhase::Attached { target, next_think } = phase else {
            return;
        };
        if next_think == 0 || self.time < next_think {
            return;
        }
        if let Some(state) = self.inner.borrow_mut().projectiles.get_mut(&self.key) {
            state.phase = Q3ProjectilePhase::Attached {
                target: target.clone(),
                next_think: 0,
            };
        }
        let Some(target) = target else {
            return;
        };
        let target_body = host.bodies().read(&target);
        let body = host.bodies().read(&self.key);
        match (target_body, body) {
            (Some(target_body), Some(body)) => {
                let point =
                    snap_vector_towards(q3_grapple_target(target_body.origin, &target_body.bounds), body.origin);
                self.base.trajectory = Trajectory {
                    base: point,
                    ..self.base.trajectory
                };
                host.bodies().write(&self.owned, BodyState { origin: point, ..body });
                host.bodies().link(&self.owned, None);
            }
            _ => self.handle().release_hook(&self.base.owner.clone()),
        }
    }

    fn base_expires(&self) -> i32 {
        self.inner
            .borrow()
            .projectiles
            .get(&self.key)
            .map_or(0, |state| state.expires)
    }
}

impl Q3ProjectileHost for StepCtx {
    fn time(&self) -> i32 {
        self.time
    }

    fn previous_time(&self) -> i32 {
        self.previous_time
    }

    fn is_live(&mut self) -> bool {
        self.host().actors().is_live(&self.key)
    }

    fn phase(&mut self) -> ProjectilePhase {
        match self.inner.borrow().projectiles.get(&self.key).map(|state| &state.phase) {
            Some(Q3ProjectilePhase::Event { .. }) => ProjectilePhase::Event,
            Some(Q3ProjectilePhase::Attached { .. }) => ProjectilePhase::Attached,
            _ => ProjectilePhase::Flight,
        }
    }

    fn event_time(&mut self) -> i32 {
        match self.inner.borrow().projectiles.get(&self.key).map(|state| &state.phase) {
            Some(Q3ProjectilePhase::Event { time, .. }) => *time,
            _ => 0,
        }
    }

    fn origin(&mut self) -> Vec3 {
        self.body().origin
    }

    fn move_to(&mut self, origin: Vec3, velocity: Vec3) {
        let body = self.body();
        self.host().bodies().write(
            &self.owned,
            BodyState {
                origin,
                velocity,
                ..body
            },
        );
    }

    fn set_origin(&mut self, point: Vec3) {
        let body = self.body();
        self.base.trajectory = Trajectory {
            trajectory_type: TrajectoryType::TrStationary,
            time: 0,
            duration: 0,
            base: point,
            delta: zero(),
        };
        self.host().bodies().write(
            &self.owned,
            BodyState {
                origin: point,
                velocity: zero(),
                ..body
            },
        );
    }

    fn link(&mut self) {
        self.host().bodies().link(&self.owned, None);
        if let Some(pending) = self.pending.take() {
            self.impact_event(&pending);
            if let Some(body) = self.host().bodies().read(&self.key) {
                let owner = self.base.owner.clone();
                self.host().weapon_impact(&owner, body.origin);
            }
        }
    }

    fn release(&mut self) {
        let owned = self.owned.clone();
        self.host().actors().release(&owned);
    }

    fn trace(&mut self, start: Vec3, end: Vec3, pass: Option<&ActorId>) -> ActorTraceResult {
        let body = self.body();
        let host = self.host();
        host.scene().trace(
            start,
            end,
            pass,
            TraceShape::Box {
                mins: body.bounds.min,
                maxs: body.bounds.max,
            },
            0x6000001,
            host.numeric(),
        )
    }

    fn target(&mut self, actor: &ActorId) -> Option<Q3ProjectileTarget> {
        let host = self.host();
        let state = host.combat().read(actor)?;
        let owner = host.combat().read(&self.base.owner);
        Some(Q3ProjectileTarget {
            damageable: state.can_take_damage,
            player: host.is_player(actor),
            invulnerable: false,
            accuracy_eligible: q3_accuracy_hit(
                host.team_game(),
                &Q3AccuracySubject {
                    actor: actor.clone(),
                    damageable: state.can_take_damage,
                    player: host.is_player(actor),
                    health: state.health,
                    team: state.team.clone(),
                },
                &Q3AccuracySubject {
                    actor: self.base.owner.clone(),
                    damageable: owner.as_ref().is_some_and(|owner| owner.can_take_damage),
                    player: host.actors().is_live(&self.base.owner) && host.is_player(&self.base.owner),
                    health: owner.as_ref().map_or(0, |owner| owner.health),
                    team: owner.as_ref().and_then(|owner| owner.team.clone()),
                },
            ),
        })
    }

    fn world_actor(&mut self) -> ActorId {
        self.host().world_actor()
    }

    fn emit(&mut self, event: &Q3ProjectileImpact) {
        match event {
            Q3ProjectileImpact::Impact {
                normal,
                target,
                flesh,
                surface_flags,
            } => {
                self.pending = Some(Q3ImpactRecord {
                    normal: *normal,
                    target: target.clone(),
                    flesh: *flesh,
                    surface_flags: *surface_flags,
                });
            }
            Q3ProjectileImpact::Bounce { normal } => {
                let origin = self.body().origin;
                let weapon = self.base.weapon;
                let key = self.key.clone();
                self.handle().emit(
                    key,
                    weapon,
                    origin,
                    origin,
                    *normal,
                    None,
                    0,
                    Q3BallisticEventKind::Bounce,
                );
            }
        }
    }

    fn retain(&mut self) {
        let pending = self
            .pending
            .clone()
            .unwrap_or_else(|| panic!("Retained Q3 projectile has no source impact event"));
        if let Some(state) = self.inner.borrow_mut().projectiles.get_mut(&self.key) {
            state.phase = Q3ProjectilePhase::Event {
                time: self.time,
                impact: pending,
            };
        }
    }

    fn damage(&mut self, target: &ActorId, direction: Vec3, point: Vec3) {
        let owner = self.base.owner.clone();
        let key = self.key.clone();
        let weapon = self.base.weapon;
        let direct = self.base.direct;
        let method = self.base.method;
        self.handle().hit(
            &owner,
            &key,
            weapon,
            target,
            point,
            direction,
            direct,
            method,
            false,
            Some(&key),
        );
    }

    fn radius(&mut self, origin: Vec3, ignore: Option<&ActorId>) -> bool {
        let base = self.base.clone();
        self.handle().radius(&base, origin, ignore)
    }

    fn accuracy(&mut self) {
        let owner = self.base.owner.clone();
        if let Some(owner) = self.host().actors().resolve_owned(&owner) {
            self.handle().credit_hit(&owner);
        }
    }

    fn think(&mut self) {
        if self.base.weapon == 10 {
            self.think_hook();
        } else if self.time >= self.base_expires() {
            // Exploding here would nest a second projectile host call
            // inside this step; the donor's think runs last in
            // `q3StepProjectile`, so deferring to post-step preserves
            // event order.
            self.inner.borrow_mut().deferred_explode = Some(self.key.clone());
        }
    }

    fn moved(&mut self) {
        let origin = self.step_origin;
        let end = self.body().origin;
        let trajectory = self.base.trajectory;
        let weapon = self.base.weapon;
        let key = self.key.clone();
        self.handle().emit(
            key,
            weapon,
            origin,
            end,
            zero(),
            None,
            0,
            Q3BallisticEventKind::Projectile { trajectory },
        );
    }

    fn has_special(&mut self) -> bool {
        self.base.weapon == 10
    }

    fn special_impact(&mut self, trace: &ActorTraceResult, target: &ActorId) -> bool {
        self.attach_hook(trace, target)
    }
}

/// Scene adapter behind radius damage.
struct SceneSpatial {
    scene: Rc<dyn Q3BallisticsScene>,
    numeric: NumericProfile,
}

impl SpatialQueries for SceneSpatial {
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId> {
        self.scene.query_actors(bounds, maximum)
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        self.scene.trace(
            query.start,
            query.end,
            query.pass_actor.as_ref(),
            query.shape,
            query.mask,
            self.numeric,
        )
    }
}

/// Radius-damage host over one projectile.
struct RadiusCtx {
    inner: Rc<RefCell<Inner>>,
    spatial: SceneSpatial,
    owner: ActorId,
    weapon: i32,
    splash: i32,
    radius: f32,
    splash_method: i32,
    projectile: ActorId,
}

impl Q3RadiusHost for RadiusCtx {
    fn spatial(&mut self) -> &mut dyn SpatialQueries {
        &mut self.spatial
    }

    fn target(&mut self, actor: &ActorId) -> Option<Q3RadiusTarget> {
        let handle = Q3SharedBallistics {
            inner: Rc::clone(&self.inner),
        };
        let host = handle.host();
        let state = host.combat().read(actor)?;
        let body = host.bodies().linked(actor)?;
        if !state.can_take_damage {
            return None;
        }
        let owner = host.combat().read(&self.owner);
        Some(Q3RadiusTarget {
            origin: body.state.origin,
            bounds: body.absolute_bounds,
            accuracy_eligible: q3_accuracy_hit(
                host.team_game(),
                &handle.accuracy_subject(actor),
                &Q3AccuracySubject {
                    actor: self.owner.clone(),
                    damageable: owner.as_ref().is_some_and(|owner| owner.can_take_damage),
                    player: host.is_player(&self.owner),
                    health: owner.as_ref().map_or(0, |owner| owner.health),
                    team: owner.as_ref().and_then(|owner| owner.team.clone()),
                },
            ),
        })
    }

    fn damage(&mut self, actor: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        let handle = Q3SharedBallistics {
            inner: Rc::clone(&self.inner),
        };
        let world = handle.host().world_actor();
        let owner = self.owner.clone();
        let weapon = self.weapon;
        let splash_method = self.splash_method;
        let projectile = self.projectile.clone();
        handle.hit(
            &owner,
            &world,
            weapon,
            actor,
            point,
            direction,
            amount,
            splash_method,
            true,
            Some(&projectile),
        );
    }
}

fn write_trajectory(trajectory: &Trajectory) -> SaveJson {
    let trajectory_type = match trajectory.trajectory_type {
        TrajectoryType::TrStationary => 0,
        TrajectoryType::TrInterpolate => 1,
        TrajectoryType::TrLinear => 2,
        TrajectoryType::TrLinearStop => 3,
        TrajectoryType::TrSine => 4,
        TrajectoryType::TrGravity => 5,
    };
    obj(vec![
        ("type", int(trajectory_type)),
        ("time", int(i64::from(trajectory.time))),
        ("duration", int(i64::from(trajectory.duration))),
        ("base", write_vector(trajectory.base)),
        ("delta", write_vector(trajectory.delta)),
    ])
}

fn write_impact(normal: Vec3, target: Option<SavedActorId>, flesh: bool, surface_flags: i32) -> SaveJson {
    obj(vec![
        ("kind", json_str("impact")),
        ("normal", write_vector(normal)),
        ("target", target.map_or(SaveJson::Null, write_saved_actor)),
        ("flesh", boolean(flesh)),
        ("surfaceFlags", int(i64::from(surface_flags))),
    ])
}

/// Serialize one projectile checkpoint.
#[must_use]
pub fn write_q3_projectile_checkpoint(state: &Q3ProjectileCheckpoint) -> SaveJson {
    let phase = match &state.phase {
        Q3ProjectileCheckpointPhase::Flight => obj(vec![("kind", json_str("flight"))]),
        Q3ProjectileCheckpointPhase::Attached { target, next_think } => obj(vec![
            ("kind", json_str("attached")),
            ("target", target.map_or(SaveJson::Null, write_saved_actor)),
            ("nextThink", int(i64::from(*next_think))),
        ]),
        Q3ProjectileCheckpointPhase::Event {
            time,
            normal,
            target,
            flesh,
            surface_flags,
        } => obj(vec![
            ("kind", json_str("event")),
            ("time", int(i64::from(*time))),
            ("impact", write_impact(*normal, *target, *flesh, *surface_flags)),
        ]),
    };
    obj(vec![
        ("actor", write_saved_actor(state.actor)),
        ("owner", write_saved_actor(state.owner)),
        ("pass", state.pass.map_or(SaveJson::Null, write_saved_actor)),
        ("damagePoint", write_vector(state.damage_point)),
        ("flags", int(i64::from(state.flags))),
        ("weapon", int(i64::from(state.weapon))),
        ("direct", int(i64::from(state.direct))),
        ("splash", int(i64::from(state.splash))),
        ("radius", int(i64::from(state.radius))),
        ("method", int(i64::from(state.method))),
        ("splashMethod", int(i64::from(state.splash_method))),
        ("expires", int(i64::from(state.expires))),
        ("trajectory", write_trajectory(&state.trajectory)),
        ("phase", phase),
    ])
}

/// Serialize one weapon statistics record.
#[must_use]
pub fn write_q3_weapon_statistics(state: &Q3WeaponStatistics) -> SaveJson {
    obj(vec![
        ("actor", write_saved_actor(SavedActorId::from(state.actor.id()))),
        ("shots", int(i64::from(state.shots))),
        ("hits", int(i64::from(state.hits))),
        ("streak", int(i64::from(state.streak))),
        ("impressiveCount", int(i64::from(state.impressive_count))),
        ("rewardUntil", int(i64::from(state.reward_until))),
    ])
}

fn read_i32(reader: SaveReader, minimum: i64) -> Result<i32, WorldError> {
    let value = reader.integer(minimum)?;
    i32::try_from(value).map_err(|_| reader.fail("expected an integer in range"))
}

fn read_f32_int(reader: SaveReader) -> Result<i32, WorldError> {
    let value = reader.finite()?;
    if value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX) {
        Ok(value as i32)
    } else {
        Err(reader.fail("expected an integer in range"))
    }
}

fn read_trajectory(reader: SaveReader) -> Result<Trajectory, WorldError> {
    let raw = reader.field("type").choice_i64(&[0, 2, 5])?;
    let raw = i32::try_from(raw).map_err(|_| reader.field("type").fail("unknown trajectory type"))?;
    let trajectory_type =
        TrajectoryType::from_i32(raw).map_err(|_| reader.field("type").fail("unknown trajectory type"))?;
    Ok(Trajectory {
        trajectory_type,
        time: read_f32_int(reader.field("time"))?,
        duration: read_f32_int(reader.field("duration"))?,
        base: read_vector(reader.field("base"))?,
        delta: read_vector(reader.field("delta"))?,
    })
}

/// Read checkpointed projectile states.
pub fn read_q3_projectile_states<'a>(
    reader: SaveReader<'a>,
    actor: &dyn Fn(SaveReader<'a>) -> Result<OwnedActor, WorldError>,
    reference: &dyn Fn(SavedActorId) -> Result<ActorId, WorldError>,
) -> Result<Vec<Q3ProjectileState>, WorldError> {
    reader.list(|value| {
        let phase_reader = value.field("phase");
        let kind = phase_reader
            .field("kind")
            .choice_str(&["flight", "event", "attached"])?;
        let phase = if kind == "flight" {
            Q3ProjectilePhase::Flight
        } else if kind == "attached" {
            Q3ProjectilePhase::Attached {
                target: phase_reader
                    .field("target")
                    .nullable(|entry| reference(read_saved_actor(entry)?))?,
                next_think: read_i32(phase_reader.field("nextThink"), 0)?,
            }
        } else {
            let impact = phase_reader.field("impact");
            impact.field("kind").literal_str("impact")?;
            Q3ProjectilePhase::Event {
                time: read_i32(phase_reader.field("time"), i64::MIN)?,
                impact: Q3ImpactRecord {
                    normal: read_vector(impact.field("normal"))?,
                    target: impact
                        .field("target")
                        .nullable(|entry| reference(read_saved_actor(entry)?))?,
                    flesh: impact.field("flesh").boolean()?,
                    surface_flags: read_i32(impact.field("surfaceFlags"), i64::MIN)?,
                },
            }
        };
        Ok(Q3ProjectileState {
            base: Q3Projectile {
                actor: actor(value.field("actor"))?.id().clone(),
                owner: reference(read_saved_actor(value.field("owner"))?)?,
                weapon: read_i32(value.field("weapon"), i64::MIN).and_then(|weapon| {
                    if matches!(weapon, 4 | 5 | 8 | 9 | 10 | 11) {
                        Ok(weapon)
                    } else {
                        Err(value.field("weapon").fail("unknown projectile weapon"))
                    }
                })?,
                direct: read_i32(value.field("direct"), 0)?,
                splash: read_i32(value.field("splash"), 0)?,
                radius: read_f32_int(value.field("radius"))?,
                method: read_i32(value.field("method"), 0)?,
                splash_method: read_i32(value.field("splashMethod"), 0)?,
                damage_point: read_vector(value.field("damagePoint"))?,
                trajectory: read_trajectory(value.field("trajectory"))?,
                flags: read_i32(value.field("flags"), i64::MIN)?,
                pass: value
                    .field("pass")
                    .nullable(|entry| reference(read_saved_actor(entry)?))?,
            },
            expires: read_f32_int(value.field("expires"))?,
            phase,
        })
    })
}

/// Read checkpointed weapon statistics.
pub fn read_q3_weapon_statistics<'a>(
    reader: SaveReader<'a>,
    actor: &dyn Fn(SaveReader<'a>) -> Result<OwnedActor, WorldError>,
) -> Result<Vec<Q3WeaponStatistics>, WorldError> {
    reader.list(|value| {
        Ok(Q3WeaponStatistics {
            actor: actor(value.field("actor"))?,
            shots: read_i32(value.field("shots"), i64::MIN)?,
            hits: read_i32(value.field("hits"), i64::MIN)?,
            streak: read_i32(value.field("streak"), i64::MIN)?,
            impressive_count: read_i32(value.field("impressiveCount"), i64::MIN)?,
            reward_until: read_i32(value.field("rewardUntil"), i64::MIN)?,
        })
    })
}

/// Encode checkpointed projectiles as one checkpoint value.
#[must_use]
pub fn encode_q3_projectile_states(states: &[Q3ProjectileCheckpoint]) -> SaveJson {
    arr(states.iter().map(write_q3_projectile_checkpoint).collect())
}

/// Encode checkpointed weapon statistics as one checkpoint value.
#[must_use]
pub fn encode_q3_weapon_statistics(states: &[Q3WeaponStatistics]) -> SaveJson {
    arr(states.iter().map(write_q3_weapon_statistics).collect())
}

/// Encode grapple-hold actors as one checkpoint value.
#[must_use]
pub fn encode_q3_hook_held(actors: &[SavedActorId]) -> SaveJson {
    arr(actors.iter().copied().map(write_saved_actor).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q3::base::records::{
        ArmorState, AttackCause, CombatState, DamageAdmissionFn, DamageOutcome, PoweredProtectionState,
        RegularArmorState,
    };
    use qa_content::q3::base::shared::definitions::{Product, EVENT_VALID_MSEC};
    use qa_content::q3::base::world::TraceSolidity;
    use qa_content::q3::foundation::arsenal::{q3_spawn_animation, q3_spawn_loadout};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Plane;
    use qa_core::numeric::{Arithmetic, FloatToInt};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_world::body::LinkedBody;
    use qa_world::movement::q3::constants::weapon;
    use qa_world::movement::types::{ActorAnimationState, MovementEnvironment, Q3UserCommand, UserCommand};
    use qa_world::registry::ActorRegistry;
    use std::cell::Cell;
    use std::collections::VecDeque;

    struct FakeBody {
        state: BodyState,
        linked: bool,
        link_count: u64,
    }

    type ReleaseCallback = Box<dyn Fn(&OwnedActor)>;

    struct FakeState {
        actors: RefCell<ActorRegistry>,
        release_cbs: RefCell<Vec<ReleaseCallback>>,
        bodies: RefCell<HashMap<ActorId, FakeBody>>,
        combat: RefCell<HashMap<ActorId, CombatState>>,
        damage_log: RefCell<Vec<DamageRequest>>,
        traces: RefCell<VecDeque<ActorTraceResult>>,
        trace_default: RefCell<ActorTraceResult>,
        area_actors: RefCell<Vec<ActorId>>,
        events: RefCell<Vec<Q3SharedBallisticEvent>>,
        steps: RefCell<HashMap<ActorId, ProjectileStep>>,
        poses: RefCell<HashMap<ActorId, Q3BallisticPose>>,
        players: RefCell<HashSet<ActorId>>,
        time: Cell<i32>,
        seed: Cell<i32>,
        team_game: bool,
        team_deathmatch: bool,
        world: ActorId,
        provider: ProviderId,
    }

    fn clear_trace(end: Vec3) -> ActorTraceResult {
        ActorTraceResult {
            fraction: 1.0,
            end,
            hit: ActorTraceHit::None,
            contact: TraceContact::None,
            solidity: TraceSolidity::Clear,
            contents: 0,
            surface_flags: 0,
        }
    }

    fn wall_trace(end: Vec3) -> ActorTraceResult {
        ActorTraceResult {
            fraction: 0.5,
            end,
            hit: ActorTraceHit::World,
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
            },
            solidity: TraceSolidity::Clear,
            contents: 1,
            surface_flags: 0,
        }
    }

    fn actor_trace(end: Vec3, actor: &ActorId) -> ActorTraceResult {
        ActorTraceResult {
            fraction: 0.5,
            end,
            hit: ActorTraceHit::Actor { actor: actor.clone() },
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
            },
            solidity: TraceSolidity::Clear,
            contents: 1,
            surface_flags: 0,
        }
    }

    fn combat_state(health: i32, team: Option<String>) -> CombatState {
        CombatState {
            health,
            armor: ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
            mass: 100,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team,
        }
    }

    struct FakeActors(Rc<FakeState>);
    struct FakeBodies(Rc<FakeState>);
    struct FakeCombat(Rc<FakeState>);
    struct FakeScene(Rc<FakeState>);

    impl Q3BallisticsActors for FakeActors {
        fn allocate(&self, provider: &ProviderId, definition: &str) -> OwnedActor {
            self.0
                .actors
                .borrow_mut()
                .allocate(provider.clone(), definition)
                .expect("fake actor capacity")
        }

        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BallisticsError> {
            self.0
                .actors
                .borrow()
                .assert_owned(actor)
                .map_err(|error| Q3BallisticsError::ActorNotOwned(error.to_string()))
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.0.actors.borrow().resolve_owned(actor)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.0.actors.borrow().is_live(actor)
        }

        fn release(&self, actor: &OwnedActor) {
            self.0
                .actors
                .borrow_mut()
                .release(actor)
                .expect("fake release of live actor");
            for callback in self.0.release_cbs.borrow().iter() {
                callback(actor);
            }
        }

        fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            self.0.release_cbs.borrow_mut().push(callback);
            Box::new(|| {})
        }
    }

    impl Q3SessionBodies for FakeBodies {
        fn create(&self, actor: &OwnedActor, state: BodyState) {
            self.0.bodies.borrow_mut().insert(
                actor.id().clone(),
                FakeBody {
                    state,
                    linked: false,
                    link_count: 0,
                },
            );
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.0.bodies.borrow().get(actor).map(|body| body.state.clone())
        }

        fn write(&self, actor: &OwnedActor, state: BodyState) {
            if let Some(body) = self.0.bodies.borrow_mut().get_mut(actor.id()) {
                body.state = state;
            }
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.0.bodies.borrow().get(actor).and_then(|body| {
                if body.linked {
                    Some(LinkedBody {
                        actor: actor.clone(),
                        state: body.state.clone(),
                        absolute_bounds: Bounds {
                            min: add3(body.state.origin, body.state.bounds.min),
                            max: add3(body.state.origin, body.state.bounds.max),
                        },
                        link_count: body.link_count,
                    })
                } else {
                    None
                }
            })
        }

        fn link(&self, actor: &OwnedActor, origin: Option<Vec3>) {
            if let Some(body) = self.0.bodies.borrow_mut().get_mut(actor.id()) {
                body.linked = true;
                body.link_count += 1;
                if let Some(origin) = origin {
                    body.state.origin = origin;
                }
            }
        }

        fn unlink(&self, actor: &OwnedActor) {
            if let Some(body) = self.0.bodies.borrow_mut().get_mut(actor.id()) {
                body.linked = false;
            }
        }
    }

    impl Q3SessionCombat for FakeCombat {
        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.0.combat.borrow().get(actor).cloned()
        }

        fn create(&self, actor: &OwnedActor, initial: CombatState, _admit: Option<DamageAdmissionFn>) {
            self.0.combat.borrow_mut().insert(actor.id().clone(), initial);
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(state) = self.0.combat.borrow_mut().get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_can_take_damage(&self, actor: &OwnedActor, can_take_damage: bool) {
            if let Some(state) = self.0.combat.borrow_mut().get_mut(actor.id()) {
                state.can_take_damage = can_take_damage;
            }
        }

        fn set_regular_points(&self, actor: &OwnedActor, _points: i32, _initial: RegularArmorState) {
            let _ = self.0.combat.borrow_mut().get_mut(actor.id());
        }

        fn bind_damage_admission(&self, _actor: &OwnedActor, _admit: DamageAdmissionFn) {}

        fn apply(&self, request: DamageRequest) -> DamageOutcome {
            self.0.damage_log.borrow_mut().push(request.clone());
            DamageOutcome::StaleTarget { request }
        }
    }

    impl Q3BallisticsScene for FakeScene {
        fn trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _pass_actor: Option<&ActorId>,
            _shape: TraceShape,
            _mask: i32,
            _numeric: NumericProfile,
        ) -> ActorTraceResult {
            let mut traces = self.0.traces.borrow_mut();
            if let Some(trace) = traces.pop_front() {
                trace
            } else {
                let mut fallback = self.0.trace_default.borrow().clone();
                fallback.end = end;
                fallback
            }
        }

        fn query_actors(&self, _bounds: &Bounds, maximum: usize) -> Vec<ActorId> {
            self.0.area_actors.borrow().iter().take(maximum).cloned().collect()
        }
    }

    struct FakeBehavior {
        launch_body: Option<BodyState>,
        step_body: Option<BodyState>,
    }

    impl Q3WeaponBehaviorPort for FakeBehavior {
        fn launch(&self, _request: &Q3BehaviorLaunch) -> Option<BodyState> {
            self.launch_body.clone()
        }

        fn step(&self, _projectile: &OwnedActor, _body: &BodyState, _time_seconds: f64) -> Option<BodyState> {
            self.step_body.clone()
        }
    }

    struct FakeHost {
        state: Rc<FakeState>,
        actors: Rc<FakeActors>,
        bodies: Rc<FakeBodies>,
        combat: Rc<FakeCombat>,
        scene: Rc<FakeScene>,
        behavior: Option<Rc<FakeBehavior>>,
    }

    impl Q3SharedBallisticsHost for FakeHost {
        fn actors(&self) -> Rc<dyn Q3BallisticsActors> {
            self.actors.clone()
        }

        fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
            self.bodies.clone()
        }

        fn scene(&self) -> Rc<dyn Q3BallisticsScene> {
            self.scene.clone()
        }

        fn combat(&self) -> Rc<dyn Q3SessionCombat> {
            self.combat.clone()
        }

        fn weapon_behavior(&self) -> Option<Rc<dyn Q3WeaponBehaviorPort>> {
            self.behavior
                .clone()
                .map(|behavior| behavior as Rc<dyn Q3WeaponBehaviorPort>)
        }

        fn weapon_provider(&self) -> ProviderId {
            self.state.provider.clone()
        }

        fn numeric(&self) -> NumericProfile {
            NumericProfile {
                id: "q3:binary32",
                arithmetic: Arithmetic::Binary32EachOp,
                float_to_int: FloatToInt::QvmIndefinite,
            }
        }

        fn pose(&self, actor: &OwnedActor) -> Q3BallisticPose {
            self.state
                .poses
                .borrow()
                .get(actor.id())
                .copied()
                .unwrap_or(Q3BallisticPose {
                    origin: zero(),
                    angles: zero(),
                    viewheight: 28.0,
                    quad: 1.0,
                    quad_active: false,
                })
        }

        fn time(&self) -> i32 {
            self.state.time.get()
        }

        fn team_deathmatch(&self) -> bool {
            self.state.team_deathmatch
        }

        fn team_game(&self) -> bool {
            self.state.team_game
        }

        fn is_player(&self, actor: &ActorId) -> bool {
            self.state.players.borrow().contains(actor)
        }

        fn world_actor(&self) -> ActorId {
            self.state.world.clone()
        }

        fn attack(
            &self,
            actor: &ActorId,
            inflictor: &ActorId,
            _weapon: i32,
            method: i32,
            flags: i32,
            originating_projectile: Option<&ActorId>,
        ) -> AttackProvenance {
            AttackProvenance {
                sequence: 0,
                time: SourceTime::Milliseconds(self.state.time.get()),
                attacker: Some(actor.clone()),
                inflictor: Some(inflictor.clone()),
                originating_projectile: originating_projectile.cloned(),
                weapon: None,
                weapon_provider: self.state.provider.clone(),
                damage_powerup_owner: None,
                combat_provider: self.state.provider.clone(),
                inventory_provider: self.state.provider.clone(),
                movement_provider: self.state.provider.clone(),
                cause: AttackCause::Q3 {
                    means_of_death: method,
                    damage_flags: flags,
                },
            }
        }

        fn event(&self, event: Q3SharedBallisticEvent) {
            self.state.events.borrow_mut().push(event);
        }

        fn track_projectile(&self, actor: &OwnedActor, _owner: &ActorId, step: ProjectileStep) {
            self.state.steps.borrow_mut().insert(actor.id().clone(), step);
        }

        fn random_seed(&self) -> i32 {
            self.state.seed.get()
        }

        fn set_random_seed(&self, seed: i32) {
            self.state.seed.set(seed);
        }
    }

    struct Fixture {
        host: Rc<FakeHost>,
        ballistics: Q3SharedBallistics,
    }

    fn fixture() -> Fixture {
        fixture_with_behavior(None)
    }

    fn fixture_with_behavior(behavior: Option<FakeBehavior>) -> Fixture {
        let owner = IdentityOwner::create("ballistics-test").unwrap();
        let world = owner.actor(1022, 1);
        let state = Rc::new(FakeState {
            actors: RefCell::new(ActorRegistry::new(owner, 64).unwrap()),
            release_cbs: RefCell::new(Vec::new()),
            bodies: RefCell::new(HashMap::new()),
            combat: RefCell::new(HashMap::new()),
            damage_log: RefCell::new(Vec::new()),
            traces: RefCell::new(VecDeque::new()),
            trace_default: RefCell::new(clear_trace(zero())),
            area_actors: RefCell::new(Vec::new()),
            events: RefCell::new(Vec::new()),
            steps: RefCell::new(HashMap::new()),
            poses: RefCell::new(HashMap::new()),
            players: RefCell::new(HashSet::new()),
            time: Cell::new(1000),
            seed: Cell::new(1),
            team_game: false,
            team_deathmatch: false,
            world,
            provider: ProviderId::new("q3", "weapon"),
        });
        let host = Rc::new(FakeHost {
            state: Rc::clone(&state),
            actors: Rc::new(FakeActors(Rc::clone(&state))),
            bodies: Rc::new(FakeBodies(Rc::clone(&state))),
            combat: Rc::new(FakeCombat(Rc::clone(&state))),
            scene: Rc::new(FakeScene(Rc::clone(&state))),
            behavior: behavior.map(Rc::new),
        });
        let ballistics = Q3SharedBallistics::new(host.clone());
        Fixture { host, ballistics }
    }

    impl Fixture {
        fn shooter(&self) -> OwnedActor {
            let actor = self.host.actors.allocate(&self.host.state.provider, "q3:player");
            self.host.bodies.create(
                &actor,
                BodyState {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: zero(),
                    velocity: zero(),
                    bounds: Bounds {
                        min: vec3(-15.0, -15.0, -24.0),
                        max: vec3(15.0, 15.0, 32.0),
                    },
                    ground: None,
                },
            );
            self.host.bodies.link(&actor, None);
            self.host.combat.create(&actor, combat_state(100, None), None);
            self.host.state.players.borrow_mut().insert(actor.id().clone());
            actor
        }

        fn victim(&self, team: Option<String>) -> OwnedActor {
            let actor = self.shooter();
            self.host.combat.create(&actor, combat_state(100, team), None);
            actor
        }

        fn input(&self, actor: &OwnedActor) -> WeaponStepInput {
            WeaponStepInput {
                actor: actor.clone(),
                command: UserCommand::Q3(Q3UserCommand {
                    server_time_milliseconds: 1000,
                    angle_words: [0, 0, 0],
                    buttons: 1,
                    weapon: weapon::MACHINEGUN,
                    forward_move: 0,
                    right_move: 0,
                    up_move: 0,
                }),
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Milliseconds(1000),
                    elapsed: SourceTime::Milliseconds(16),
                    phase: FramePhase::FrameEntry,
                },
                arsenal: q3_spawn_loadout(self.host.state.provider.clone(), Product::Baseq3, false),
                animation: ActorAnimationState {
                    provider: self.host.state.provider.clone(),
                    state: q3_spawn_animation(),
                },
                environment: MovementEnvironment {
                    health: 100.0,
                    ..Default::default()
                },
                gauntlet_hit: false,
            }
        }

        fn queue(&self, trace: ActorTraceResult) {
            self.host.state.traces.borrow_mut().push_back(trace);
        }

        fn step_all(&self, previous: i32, time: i32) {
            self.host.state.time.set(time);
            let steps: Vec<(ActorId, ProjectileStep)> = self
                .host
                .state
                .steps
                .borrow()
                .iter()
                .map(|(actor, step)| (actor.clone(), step.clone()))
                .collect();
            for (_, step) in steps {
                step(previous, time).unwrap();
            }
        }

        fn kinds(&self) -> Vec<String> {
            self.host
                .state
                .events
                .borrow()
                .iter()
                .map(|event| match &event.kind {
                    Q3BallisticEventKind::Remove => "remove".to_string(),
                    Q3BallisticEventKind::Bounce => "bounce".to_string(),
                    Q3BallisticEventKind::Trail => "trail".to_string(),
                    Q3BallisticEventKind::Fire { .. } => "fire".to_string(),
                    Q3BallisticEventKind::Projectile { .. } => "projectile".to_string(),
                    Q3BallisticEventKind::Impact { hit_kind } => match hit_kind {
                        Q3ImpactKind::Wall => "impact-wall".to_string(),
                        Q3ImpactKind::Flesh => "impact-flesh".to_string(),
                    },
                    Q3BallisticEventKind::Contact { .. } => "contact".to_string(),
                    Q3BallisticEventKind::Shotgun { .. } => "shotgun".to_string(),
                    Q3BallisticEventKind::Rail { .. } => "rail".to_string(),
                    Q3BallisticEventKind::RailAward { .. } => "rail-award".to_string(),
                })
                .collect()
        }
    }

    #[test]
    fn machinegun_counts_shot_and_impacts_wall() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.queue(wall_trace(vec3(100.0, 0.0, 0.0)));
        fixture.ballistics.fire(&shooter, 2, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.ballistics.weapon_statistics(&shooter).shots, 1);
        assert_eq!(fixture.kinds(), vec!["fire", "impact-wall"]);
        assert!(fixture.host.state.damage_log.borrow().is_empty());
    }

    #[test]
    fn machinegun_damages_actor_target() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let victim = fixture.victim(None);
        fixture.queue(actor_trace(vec3(100.0, 0.0, 0.0), victim.id()));
        fixture.ballistics.fire(&shooter, 2, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.kinds(), vec!["fire", "impact-flesh"]);
        assert_eq!(fixture.host.state.damage_log.borrow().len(), 1);
        assert_eq!(fixture.ballistics.weapon_statistics(&shooter).hits, 1);
    }

    #[test]
    fn shotgun_emits_pellet_event() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 3, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.ballistics.weapon_statistics(&shooter).shots, 1);
        assert!(fixture.kinds().contains(&"shotgun".to_string()));
    }

    #[test]
    fn lightning_emits_trail() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.queue(wall_trace(vec3(100.0, 0.0, 0.0)));
        fixture.ballistics.fire(&shooter, 6, &fixture.input(&shooter)).unwrap();
        assert!(fixture.kinds().contains(&"trail".to_string()));
    }

    #[test]
    fn rail_awards_impressive_on_second_hit() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let victim = fixture.victim(None);
        for _ in 0..2 {
            fixture.queue(actor_trace(vec3(100.0, 0.0, 0.0), victim.id()));
            fixture.ballistics.fire(&shooter, 7, &fixture.input(&shooter)).unwrap();
        }
        let stats = fixture.ballistics.weapon_statistics(&shooter);
        assert_eq!(stats.shots, 2);
        assert!(fixture.kinds().contains(&"rail-award".to_string()));
    }

    #[test]
    fn gauntlet_reports_contact() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        assert!(!fixture.ballistics.gauntlet_hit(&shooter));
        assert!(fixture.host.state.events.borrow().is_empty());
        let victim = fixture.victim(None);
        fixture.queue(actor_trace(vec3(10.0, 0.0, 0.0), victim.id()));
        assert!(fixture.ballistics.gauntlet_hit(&shooter));
        assert_eq!(fixture.kinds(), vec!["contact"]);
        assert_eq!(fixture.host.state.damage_log.borrow().len(), 1);
    }

    #[test]
    fn rocket_impacts_wall_then_releases() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 5, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.ballistics.checkpoint().len(), 1);
        fixture.queue(wall_trace(vec3(50.0, 0.0, 0.0)));
        fixture.step_all(1000, 1050);
        assert!(fixture.kinds().contains(&"impact-wall".to_string()));
        assert_eq!(fixture.ballistics.checkpoint().len(), 1);
        fixture.step_all(1050, 1050 + EVENT_VALID_MSEC + 1);
        assert!(fixture.kinds().contains(&"remove".to_string()));
        assert!(fixture.ballistics.checkpoint().is_empty());
    }

    #[test]
    fn nailgun_launches_fifteen() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 11, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.ballistics.weapon_statistics(&shooter).shots, 15);
        assert_eq!(fixture.ballistics.checkpoint().len(), 15);
    }

    #[test]
    fn hook_attaches_pulls_and_releases() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let victim = fixture.victim(None);
        fixture.ballistics.fire(&shooter, 10, &fixture.input(&shooter)).unwrap();
        assert_eq!(fixture.ballistics.checkpoint_hook_held().len(), 1);
        fixture.queue(actor_trace(vec3(50.0, 0.0, 0.0), victim.id()));
        fixture.step_all(1000, 1050);
        assert!(fixture.ballistics.grapple_point(shooter.id()).is_some());
        assert!(fixture.ballistics.pull(&shooter).is_some());
        fixture.ballistics.command(&shooter, false, true, true);
        assert!(fixture.ballistics.grapple_point(shooter.id()).is_none());
        assert!(fixture.kinds().contains(&"remove".to_string()));
    }

    #[test]
    fn grapple_expires_in_flight() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 10, &fixture.input(&shooter)).unwrap();
        fixture.step_all(1000, 1000 + Q3_GRAPPLE_LIFETIME + 100);
        assert!(fixture.ballistics.checkpoint().is_empty());
        assert!(fixture.kinds().contains(&"remove".to_string()));
    }

    #[test]
    fn checkpoint_restores_projectiles() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 2, &fixture.input(&shooter)).unwrap();
        fixture.ballistics.fire(&shooter, 5, &fixture.input(&shooter)).unwrap();
        fixture.step_all(1000, 1050);
        let checkpoints = fixture.ballistics.checkpoint();
        assert_eq!(checkpoints.len(), 1);
        let value = encode_q3_projectile_states(&checkpoints);
        let live: HashMap<SavedActorId, ActorId> = [shooter.id().clone()]
            .into_iter()
            .chain(fixture.host.state.steps.borrow().keys().cloned())
            .map(|id| (SavedActorId::from(&id), id))
            .collect();
        let restored = read_q3_projectile_states(
            SaveReader::new(&value),
            &|entry| {
                let saved = read_saved_actor(entry)?;
                live.get(&saved)
                    .and_then(|id| fixture.host.actors.resolve_owned(id))
                    .ok_or_else(|| SaveReader::new(&value).fail("unknown checkpoint actor"))
            },
            &|saved| {
                live.get(&saved)
                    .cloned()
                    .ok_or_else(|| WorldError::BadSave("unknown checkpoint actor".to_string()))
            },
        )
        .unwrap();
        fixture.ballistics.restore(restored).unwrap();
        let mut before = checkpoints;
        let mut after = fixture.ballistics.checkpoint();
        before.sort_by_key(|state| (state.actor.slot, state.actor.generation));
        after.sort_by_key(|state| (state.actor.slot, state.actor.generation));
        assert_eq!(before, after);
    }

    #[test]
    fn weapon_statistics_round_trip() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.queue(wall_trace(vec3(100.0, 0.0, 0.0)));
        fixture.ballistics.fire(&shooter, 2, &fixture.input(&shooter)).unwrap();
        let stats = fixture.ballistics.checkpoint_weapon_statistics();
        assert_eq!(stats.len(), 1);
        let value = encode_q3_weapon_statistics(&stats);
        let restored = read_q3_weapon_statistics(SaveReader::new(&value), &|entry| {
            let saved = read_saved_actor(entry)?;
            assert_eq!(saved, SavedActorId::from(shooter.id()));
            Ok(shooter.clone())
        })
        .unwrap();
        fixture.ballistics.restore_weapon_statistics(restored).unwrap();
        assert_eq!(fixture.ballistics.checkpoint_weapon_statistics(), stats);
    }

    #[test]
    fn hook_held_round_trip() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        fixture.ballistics.fire(&shooter, 10, &fixture.input(&shooter)).unwrap();
        let held = fixture.ballistics.checkpoint_hook_held();
        assert_eq!(held, vec![SavedActorId::from(shooter.id())]);
        let value = encode_q3_hook_held(&held);
        let restored: Vec<OwnedActor> = SaveReader::new(&value)
            .list(|entry| {
                let saved = read_saved_actor(entry)?;
                assert_eq!(saved, SavedActorId::from(shooter.id()));
                Ok::<OwnedActor, WorldError>(shooter.clone())
            })
            .unwrap();
        fixture.ballistics.restore_hook_held(&restored).unwrap();
        assert_eq!(fixture.ballistics.checkpoint_hook_held(), held);
    }

    #[test]
    fn respawn_resets_streak() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let victim = fixture.victim(None);
        for _ in 0..2 {
            fixture.queue(actor_trace(vec3(100.0, 0.0, 0.0), victim.id()));
            fixture.ballistics.fire(&shooter, 7, &fixture.input(&shooter)).unwrap();
        }
        fixture.ballistics.respawn(shooter.id());
        let stats = fixture.ballistics.weapon_statistics(&shooter);
        assert_eq!(stats.streak, 0);
        assert_eq!(stats.reward_until, 0);
    }

    #[test]
    fn unsupported_weapon_errors_after_counting() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let input = fixture.input(&shooter);
        let error = fixture.ballistics.fire(&shooter, 99, &input).unwrap_err();
        assert_eq!(error, Q3BallisticsError::UnsupportedWeapon(99));
        assert_eq!(fixture.ballistics.weapon_statistics(&shooter).shots, 1);
    }

    #[test]
    fn duplicate_hook_restore_errors() {
        let fixture = fixture();
        let shooter = fixture.shooter();
        let hook = |fixture: &Fixture| {
            let actor = fixture
                .host
                .actors
                .allocate(&fixture.host.state.provider, "q3:projectile");
            Q3ProjectileState {
                base: Q3Projectile {
                    actor: actor.id().clone(),
                    owner: shooter.id().clone(),
                    weapon: 10,
                    direct: 0,
                    splash: 0,
                    radius: 0,
                    method: 23,
                    splash_method: 0,
                    damage_point: zero(),
                    trajectory: Trajectory::zero(TrajectoryType::TrLinear),
                    flags: 0,
                    pass: Some(shooter.id().clone()),
                },
                expires: 11000,
                phase: Q3ProjectilePhase::Flight,
            }
        };
        let error = fixture
            .ballistics
            .restore(vec![hook(&fixture), hook(&fixture)])
            .unwrap_err();
        assert_eq!(error, Q3BallisticsError::DuplicateHook);
    }

    #[test]
    fn behavior_launch_override_applies() {
        let origin = vec3(5.0, 6.0, 7.0);
        let velocity = vec3(1.0, 2.0, 3.0);
        let armed = fixture_with_behavior(Some(FakeBehavior {
            launch_body: Some(BodyState {
                origin,
                angles: zero(),
                velocity,
                bounds: Bounds {
                    min: zero(),
                    max: zero(),
                },
                ground: None,
            }),
            step_body: None,
        }));
        let shooter = armed.shooter();
        armed.ballistics.fire(&shooter, 5, &armed.input(&shooter)).unwrap();
        let checkpoints = armed.ballistics.checkpoint();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].trajectory.base, origin);
        assert_eq!(checkpoints[0].trajectory.delta, velocity);
    }

    struct CanonicalBehavior {
        update: Option<WeaponTrajectoryUpdate>,
    }

    impl WeaponBehaviorProjectilePort for CanonicalBehavior {
        fn controls_trajectory(&self, _projectile: &ActorId) -> bool {
            true
        }

        fn launch(&mut self, _input: &WeaponBehaviorLaunch) -> Option<WeaponTrajectoryUpdate> {
            self.update
        }

        fn step(
            &mut self,
            _projectile: &OwnedActor,
            _body: &ContractBodyState,
            _time_seconds: f64,
        ) -> Option<WeaponTrajectoryUpdate> {
            self.update
        }
    }

    #[test]
    fn canonical_port_adapts_onto_ballistics() {
        let armed = fixture();
        let shooter = armed.shooter();
        let body = BodyState {
            origin: vec3(1.0, 2.0, 3.0),
            angles: zero(),
            velocity: zero(),
            bounds: Bounds {
                min: zero(),
                max: zero(),
            },
            ground: None,
        };
        let update = WeaponTrajectoryUpdate {
            origin: vec3(5.0, 6.0, 7.0),
            velocity: vec3(1.0, 2.0, 3.0),
            angles: vec3(0.0, 90.0, 0.0),
        };
        let port = CanonicalWeaponBehaviorPort(RefCell::new(CanonicalBehavior { update: Some(update) }));
        let launched = port
            .launch(&Q3BehaviorLaunch {
                projectile: shooter.clone(),
                shooter: shooter.id().clone(),
                weapon: "q3:weapon/rocketlauncher".to_string(),
                role: ProjectileRole::Rocket,
                time_seconds: 1.0,
                body: body.clone(),
            })
            .unwrap();
        assert_eq!(launched.origin, update.origin);
        assert_eq!(launched.velocity, update.velocity);
        assert_eq!(launched.angles, update.angles);
        assert_eq!(launched.bounds, body.bounds);
        let stepped = port.step(&shooter, &body, 2.0).unwrap();
        assert_eq!(stepped.origin, update.origin);
        assert_eq!(stepped.ground, None);
    }
}
