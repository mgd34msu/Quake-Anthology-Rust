//! Q1 foundation host seam (`Q1FoundationHost`, donor
//! `src/content/q1/foundation/types.ts`).
//!
//! The host owns the shared session tables (actors, bodies, combat,
//! inventory) plus engine builtins (trace, scheduling, presentation).
//! Table traits below mirror the surface Q1 content actually uses;
//! donors: `src/world/actors/registry.ts`, `src/world/actors/body.ts`,
//! `src/world/actors/callbacks.ts`, `src/world/gameplay/authority.ts`,
//! and `src/world/gameplay/inventory.ts`.
//!
//! Two deliberate seam changes versus the donor, both forced by Rust
//! ownership rather than behavior:
//!
//! - Actor release runs through
//!   [`Q1EntityServices::release_actor`](super::entity_services::Q1EntityServices::release_actor),
//!   which releases the shared actor and then runs game-registered
//!   [`Q1ReleaseHook`]s. The donor's `actors.onRelease` callbacks cannot
//!   borrow the services object they are stored in; routing every
//!   release through the game preserves the synchronous cleanup order.
//! - Pusher stepping is a single [`Q1FoundationHost::step_pusher`] engine
//!   call. The donor's `pusherServices(game)` bundle closes over game
//!   state the engine cannot borrow; the engine runs the
//!   `movement/q1` pusher transaction (donor
//!   `src/movement/q1/pusher.ts`) and reports back moved/blocked
//!   actors, so the movement logic is neither duplicated nor deferred.

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::{
    ArmorState, InventoryEntry, ItemId, OriginalPickupAdmission, OriginalPickupContinuation, OriginalPickupOffer,
    OriginalPickupOutcome, ProjectileRole, RegularArmorState,
};
use crate::monsters::MonsterTargetObservation;

use super::entity_services::Q1EntityServices;
use super::gameplay::{
    ActorObservation, BodyAttachment, BodyState, CombatState, CombatTraits, DamageOutcome, DamageRequest,
    SourceDamageModifier, SourceSlot, TransitionIntent,
};
use super::types::{Q1Event, Q1Powerup, Q1Trace, Q1TraceRequest};
use crate::q1::{q1_error, Q1Error};

/// Contents classification (`contents` return, donor `types.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1Contents {
    /// Empty space.
    Empty,
    /// Solid geometry.
    Solid,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
    /// Sky.
    Sky,
}

/// Monster goal approach mode (`moveToGoal` mode, donor `types.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1GoalMode {
    /// Stop in range.
    Range,
    /// Move into contact.
    Contact,
}

/// Observed source damage traits (`sourceTarget` return, donor
/// `types.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q1SourceTarget {
    /// Whether the actor takes aimed damage.
    pub aimed_damage: bool,
    /// Whether the actor is a pusher.
    pub push: bool,
    /// Whether the actor is a player.
    pub player: bool,
    /// Whether the actor is a slidebox.
    pub slidebox: bool,
}

/// Player cinematic control (`controlPlayer` control, donor `types.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1CutsceneControl {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera angles.
    pub angles: Vec3,
    /// View offset.
    pub view_offset: Vec3,
}

/// Pusher step status (`Q1PusherResult["status"]`, donor
/// `movement/q1/types.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PusherStatus {
    /// The pusher moved.
    Moved,
    /// The pusher was blocked.
    Blocked,
    /// The pusher actor was removed.
    ActorRemoved,
}

/// Pusher step result (`Q1PusherResult`, donor `movement/q1/types.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1PusherStep {
    /// Pusher actor.
    pub actor: ActorId,
    /// Step status.
    pub status: Q1PusherStatus,
    /// Actors displaced by the step.
    pub moved: Vec<ActorId>,
}

/// Damage adjustment (`adjustDamage` return, donor
/// `world/gameplay/authority.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageAdjust {
    /// Adjusted amount.
    pub amount: f64,
    /// Adjusted knockback.
    pub knockback: f64,
}

/// Shared session actor registry used by Q1 content (donor
/// `src/world/actors/registry.ts`). Release callbacks are game-routed
/// (see module docs); the registry itself stays a plain table.
pub trait Q1SessionActorRegistry {
    /// Allocate an actor at a source slot.
    fn allocate_at_source(
        &mut self,
        owner: &ProviderId,
        source_slot: u32,
        definition: &str,
    ) -> Result<OwnedActor, Q1Error>;
    /// Assert session ownership of an actor handle.
    fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q1Error>;
    /// Resolve a live owned handle for an actor id.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Whether the actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Observe every allocated actor.
    fn observations(&self) -> Vec<ActorObservation>;
    /// Source slot binding of an actor, if any.
    fn source_of(&self, actor: &ActorId) -> Option<SourceSlot>;
    /// Release an actor.
    fn release(&mut self, actor: &OwnedActor) -> Result<(), Q1Error>;
    /// Resolve a saved actor reference.
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor>;
    /// Reference a saved actor in the current session.
    fn reference_saved(&mut self, saved: &SavedActorId) -> ActorId;
}

/// Shared body table used by Q1 content (donor
/// `src/world/actors/body.ts`).
pub trait Q1SharedBodyTable {
    /// Admit an actor body.
    fn create(&mut self, actor: &OwnedActor, initial: &BodyState) -> Result<(), Q1Error>;
    /// Read an actor body.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write an actor body.
    fn write(&mut self, actor: &OwnedActor, state: &BodyState) -> Result<(), Q1Error>;
    /// Link an actor body for spatial queries.
    fn link(&mut self, actor: &OwnedActor) -> Result<(), Q1Error>;
    /// Read the last linked body snapshot.
    fn linked(&self, actor: &ActorId) -> Option<super::gameplay::LinkedBody>;
    /// Attach a body to an anchor.
    fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) -> Result<(), Q1Error>;
    /// Detach a body from its anchor.
    fn detach(&mut self, actor: &OwnedActor) -> Result<(), Q1Error>;
    /// Drop a body on actor release.
    fn remove(&mut self, _actor: &ActorId) {}
}

/// Shared combat authority used by Q1 content (donor
/// `src/world/gameplay/authority.ts`).
pub trait Q1GameplayAuthority {
    /// Admit actor combat state.
    fn create(&mut self, actor: &OwnedActor, initial: &CombatState) -> Result<(), Q1Error>;
    /// Read actor combat state.
    fn read(&self, actor: &ActorId) -> Option<CombatState>;
    /// Write actor health.
    fn set_health(&mut self, actor: &OwnedActor, health: f64) -> Result<(), Q1Error>;
    /// Write actor armor.
    fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) -> Result<(), Q1Error>;
    /// Write actor traits.
    fn set_traits(&mut self, actor: &OwnedActor, traits: CombatTraits) -> Result<(), Q1Error>;
    /// Write regular armor.
    fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState) -> Result<(), Q1Error>;
    /// Write regular armor points.
    fn set_regular_points(&mut self, actor: &OwnedActor, points: f64) -> Result<(), Q1Error>;
    /// Bind a foreign-damage adjustment hook.
    fn bind_damage_adjustment(&mut self, actor: &OwnedActor, adjust: Q1DamageAdjustHook);
    /// Apply a damage request.
    fn apply(&mut self, request: &DamageRequest) -> DamageOutcome;
}

/// Shared inventory table used by Q1 content (donor
/// `src/world/gameplay/inventory.ts`).
pub trait Q1SharedInventoryTable {
    /// Admit actor inventory entries.
    fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) -> Result<(), Q1Error>;
    /// Read admitted entries (`[]` when unbound, per the donor).
    fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry>;
    /// Whether the actor has an inventory binding.
    fn has(&self, actor: &ActorId) -> bool;
    /// Count an item (`0` when unbound, per the donor).
    fn count(&self, actor: &ActorId, item: &ItemId) -> f64;
    /// Consume items, reporting success.
    fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool;
    /// Give items, reporting the granted count.
    fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64;
    /// Configure (write) an entry.
    fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) -> Result<(), Q1Error>;
    /// Adjust a signed source counter past zero.
    fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> Result<f64, Q1Error>;
}

/// Standard Q1 actor callback bindings. Every Q1 entity receives the
/// same binding (think/touch/use/pain/die dispatch through the named
/// source callbacks); the table only tracks which actors are bound.
/// The engine drives thinks through
/// [`Q1EntityServices::fire_think`](super::entity_services::Q1EntityServices::fire_think)
/// and the sibling `fire_*` entry points.
pub trait Q1ActorCallbackTable {
    /// Bind the standard Q1 callbacks for an actor.
    fn bind(&mut self, actor: &OwnedActor);
    /// Remove an actor binding.
    fn unbind(&mut self, actor: &OwnedActor);
    /// Whether the actor is bound.
    fn is_bound(&self, actor: &ActorId) -> bool;
}

/// View punch-angle store (`punchAngles`, donor `types.ts`).
pub trait Q1PunchAngles {
    /// Read punch angles.
    fn read(&mut self, actor: &ActorId) -> Vec3;
    /// Write punch angles.
    fn write(&mut self, actor: &ActorId, angles: Vec3);
}

/// Projectile trajectory update (`WeaponTrajectoryUpdate`, donor
/// `contracts/weapon-behavior.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1TrajectoryUpdate {
    /// Updated origin.
    pub origin: Vec3,
    /// Updated velocity.
    pub velocity: Vec3,
    /// Updated angles.
    pub angles: Vec3,
}

/// Projectile behavior launch (`WeaponBehaviorLaunch`, donor
/// `contracts/weapon-behavior.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WeaponBehaviorLaunch {
    /// Projectile actor.
    pub projectile: OwnedActor,
    /// Shooter.
    pub shooter: ActorId,
    /// Weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Projectile behavior port (`WeaponBehaviorProjectilePort`, donor
/// `contracts/weapon-behavior.ts`).
pub trait Q1WeaponBehaviorPort {
    /// Whether the port controls a projectile trajectory.
    fn controls_trajectory(&mut self, projectile: &ActorId) -> bool;
    /// Launch a projectile behavior.
    fn launch(&mut self, input: &Q1WeaponBehaviorLaunch) -> Option<Q1TrajectoryUpdate>;
    /// Step a projectile behavior.
    fn step(&mut self, projectile: &OwnedActor, body: &BodyState, time_seconds: f64) -> Option<Q1TrajectoryUpdate>;
}

/// Synchronous actor-release cleanup (`actors.onRelease` callbacks,
/// donor `world/actors/registry.ts`). Hooks run inside
/// [`Q1EntityServices::release_actor`](super::entity_services::Q1EntityServices::release_actor)
/// after the shared release, with full game access.
pub trait Q1ReleaseHook {
    /// Clean up module state for a released actor.
    fn on_release(&mut self, game: &mut Q1EntityServices, actor: &OwnedActor);
}

/// Entity registration hook (`registerEntity`, donor `types.ts`).
pub trait Q1RegisterEntity {
    /// Observe a newly admitted source entity.
    fn register(&mut self, game: &mut Q1EntityServices, id: &ActorId);
}

/// Object-safe original-pickup admission port. The contract
/// [`OriginalPickupAdmission`] is not `dyn`-compatible (generic
/// `run_source`); the foundation only touches, so the port carries the
/// touch entry and any provider adapts automatically.
pub trait Q1OriginalPickupPort {
    /// Touch a pickup.
    fn touch(
        &self,
        offer: &OriginalPickupOffer,
        continuation: &dyn OriginalPickupContinuation,
    ) -> OriginalPickupOutcome;
}

impl<T: OriginalPickupAdmission> Q1OriginalPickupPort for T {
    fn touch(
        &self,
        offer: &OriginalPickupOffer,
        continuation: &dyn OriginalPickupContinuation,
    ) -> OriginalPickupOutcome {
        OriginalPickupAdmission::touch(self, offer, continuation)
    }
}

/// Unit random draw hook.
pub type Q1RandomHook = Box<dyn FnMut() -> f64>;
/// Collision trace hook.
pub type Q1TraceHook = Box<dyn FnMut(&Q1TraceRequest) -> Q1Trace>;
/// Point contents classification hook.
pub type Q1ContentsHook = Box<dyn FnMut(Vec3) -> Q1Contents>;
/// Monster step move hook.
pub type Q1WalkMoveHook = Box<dyn FnMut(&OwnedActor, f64, f64) -> bool>;
/// Monster yaw update hook.
pub type Q1ChangeYawHook = Box<dyn FnMut(&OwnedActor)>;
/// Monster goal move hook.
pub type Q1MoveToGoalHook = Box<dyn FnMut(&OwnedActor, &ActorId, f64, Option<Q1GoalMode>)>;
/// Monster ground check hook.
pub type Q1CheckBottomHook = Box<dyn FnMut(&ActorId) -> bool>;
/// Actor think scheduler hook.
pub type Q1ScheduleThinkHook = Box<dyn FnMut(&OwnedActor, f64)>;
/// Actor think cancel hook.
pub type Q1CancelThinkHook = Box<dyn FnMut(&OwnedActor)>;
/// Presentation event hook.
pub type Q1EmitHook = Box<dyn FnMut(Q1Event)>;
/// Session transition hook.
pub type Q1TransitionHook = Box<dyn FnMut(TransitionIntent)>;
/// Admitted players hook.
pub type Q1PlayersHook = Box<dyn FnMut() -> Vec<ActorId>>;
/// Visible client cycle hook.
pub type Q1CheckClientHook = Box<dyn FnMut(&OwnedActor) -> Option<ActorId>>;
/// Gameplay classname hook.
pub type Q1ClassnameHook = Box<dyn FnMut(&ActorId) -> String>;
/// Timed powerup hook.
pub type Q1PowerupHook = Box<dyn FnMut(&OwnedActor, Q1Powerup, f64)>;
/// Pusher step hook.
pub type Q1StepPusherHook = Box<dyn FnMut(&ActorId, f64) -> Q1PusherStep>;
/// Weapon impact effect hook.
pub type Q1WeaponImpactHook = Box<dyn FnMut(&ActorId, Vec3)>;
/// Weapon volume hook.
pub type Q1WeaponVolumeHook = Box<dyn FnMut(&ActorId) -> f64>;
/// Monster target observation hook.
pub type Q1MonsterTargetHook = Box<dyn FnMut(&ActorId) -> Option<MonsterTargetObservation>>;
/// Gravity scale hook.
pub type Q1SetGravityHook = Box<dyn FnMut(&ActorId, f64)>;
/// Player cinematic control hook.
pub type Q1ControlPlayerHook = Box<dyn FnMut(&ActorId, &Q1CutsceneControl)>;
/// Source target observation hook.
pub type Q1SourceTargetHook = Box<dyn FnMut(&ActorId) -> Q1SourceTarget>;
/// Powerup expiry hook.
pub type Q1PowerupExpiresHook = Box<dyn FnMut(&ActorId, Q1Powerup) -> f64>;
/// Source damage multiplier hook.
pub type Q1SourceDamageMultiplierHook = Box<dyn FnMut(&ActorId) -> f64>;
/// Foreign-damage adjustment hook.
pub type Q1DamageAdjustHook = Box<dyn Fn(&DamageRequest) -> Option<DamageAdjust>>;

/// Engine host for Q1 content (`Q1FoundationHost`, donor
/// `src/content/q1/foundation/types.ts`). Required builtins are boxed
/// closures; optional donor hooks are `Option`s.
pub struct Q1FoundationHost {
    /// Shared actor registry.
    pub actors: Box<dyn Q1SessionActorRegistry>,
    /// Shared body table.
    pub bodies: Box<dyn Q1SharedBodyTable>,
    /// Shared callback bindings.
    pub callbacks: Box<dyn Q1ActorCallbackTable>,
    /// Shared combat authority.
    pub combat: Box<dyn Q1GameplayAuthority>,
    /// Shared inventory table.
    pub inventory: Box<dyn Q1SharedInventoryTable>,
    /// Original pickup admission, when the session qualifies map grants.
    pub original_pickups: Option<Box<dyn Q1OriginalPickupPort>>,
    /// View punch-angle store.
    pub punch_angles: Option<Box<dyn Q1PunchAngles>>,
    /// Projectile behavior port.
    pub weapon_behavior: Option<Box<dyn Q1WeaponBehaviorPort>>,
    /// Entity registration hook.
    pub register_entity: Option<Box<dyn Q1RegisterEntity>>,
    /// Source damage modifier.
    pub source_damage_modifier: Option<SourceDamageModifier>,
    /// Source damage powerup owner.
    pub source_damage_powerup_owner: Option<ProviderId>,
    /// Unit random draw.
    pub random: Q1RandomHook,
    /// Collision trace.
    pub trace: Q1TraceHook,
    /// Point contents classification.
    pub contents: Q1ContentsHook,
    /// Monster step move.
    pub walk_move: Q1WalkMoveHook,
    /// Monster yaw update.
    pub change_yaw: Q1ChangeYawHook,
    /// Monster goal move.
    pub move_to_goal: Q1MoveToGoalHook,
    /// Monster ground check.
    pub check_bottom: Q1CheckBottomHook,
    /// Schedule an actor think.
    pub schedule_think: Q1ScheduleThinkHook,
    /// Cancel an actor think.
    pub cancel_think: Q1CancelThinkHook,
    /// Emit a presentation event.
    pub emit: Q1EmitHook,
    /// Request a session transition.
    pub transition: Q1TransitionHook,
    /// Admitted players.
    pub players: Q1PlayersHook,
    /// Visible client cycle.
    pub check_client: Q1CheckClientHook,
    /// Gameplay classname of an actor.
    pub classname: Q1ClassnameHook,
    /// Apply a timed powerup to shared player state.
    pub powerup: Q1PowerupHook,
    /// Step a pusher through the movement lane.
    pub step_pusher: Q1StepPusherHook,
    /// Weapon impact effect hook.
    pub weapon_impact: Option<Q1WeaponImpactHook>,
    /// Weapon volume hook.
    pub weapon_volume: Option<Q1WeaponVolumeHook>,
    /// Monster target observation hook.
    pub monster_target: Option<Q1MonsterTargetHook>,
    /// Gravity scale hook.
    pub set_gravity: Option<Q1SetGravityHook>,
    /// Player cinematic control hook.
    pub control_player: Option<Q1ControlPlayerHook>,
    /// Source target observation hook.
    pub source_target: Option<Q1SourceTargetHook>,
    /// Powerup expiry hook.
    pub powerup_expires: Option<Q1PowerupExpiresHook>,
    /// Source damage multiplier hook.
    pub source_damage_multiplier: Option<Q1SourceDamageMultiplierHook>,
}

impl std::fmt::Debug for Q1FoundationHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Q1FoundationHost").finish_non_exhaustive()
    }
}

impl Q1FoundationHost {
    /// Draw a unit random value.
    pub fn random(&mut self) -> f64 {
        (self.random)()
    }

    /// Run a collision trace.
    pub fn trace(&mut self, request: &Q1TraceRequest) -> Q1Trace {
        (self.trace)(request)
    }

    /// Classify point contents.
    pub fn contents(&mut self, point: Vec3) -> Q1Contents {
        (self.contents)(point)
    }

    /// Emit a presentation event.
    pub fn emit(&mut self, event: Q1Event) {
        (self.emit)(event)
    }

    /// Request a session transition.
    pub fn transition(&mut self, intent: TransitionIntent) {
        (self.transition)(intent)
    }

    /// Schedule an actor think.
    pub fn schedule_think(&mut self, actor: &OwnedActor, due: f64) {
        (self.schedule_think)(actor, due);
    }

    /// Cancel an actor think.
    pub fn cancel_think(&mut self, actor: &OwnedActor) {
        (self.cancel_think)(actor);
    }

    /// Gameplay classname of an actor.
    pub fn classname(&mut self, actor: &ActorId) -> String {
        (self.classname)(actor)
    }

    /// Apply a timed powerup to shared player state.
    pub fn powerup(&mut self, actor: &OwnedActor, powerup: Q1Powerup, expires: f64) {
        (self.powerup)(actor, powerup, expires);
    }

    /// Step a pusher through the movement lane.
    pub fn step_pusher(&mut self, actor: &ActorId, elapsed: f64) -> Q1PusherStep {
        (self.step_pusher)(actor, elapsed)
    }

    /// Run a monster step move.
    pub fn walk_move(&mut self, actor: &OwnedActor, yaw: f64, distance: f64) -> bool {
        (self.walk_move)(actor, yaw, distance)
    }

    /// Update a monster yaw.
    pub fn change_yaw(&mut self, actor: &OwnedActor) {
        (self.change_yaw)(actor);
    }

    /// Move a monster toward a goal.
    pub fn move_to_goal(&mut self, actor: &OwnedActor, goal: &ActorId, distance: f64, mode: Option<Q1GoalMode>) {
        (self.move_to_goal)(actor, goal, distance, mode);
    }

    /// Check a monster has ground below.
    pub fn check_bottom(&mut self, actor: &ActorId) -> bool {
        (self.check_bottom)(actor)
    }

    /// Cycle the visible client for monster sighting.
    pub fn check_client(&mut self, actor: &OwnedActor) -> Option<ActorId> {
        (self.check_client)(actor)
    }

    /// Require the gravity hook (donor throws without a movement host).
    pub fn require_set_gravity(&mut self, actor: &ActorId, scale: f64) -> Result<(), Q1Error> {
        match self.set_gravity.as_mut() {
            Some(set_gravity) => {
                set_gravity(actor, scale);
                Ok(())
            }
            None => Err(q1_error(
                "Q1 source gravity mutation requires the selected movement host",
            )),
        }
    }

    /// Require the player-control hook (donor throws without a control
    /// host).
    pub fn require_control_player(&mut self, actor: &ActorId, control: &Q1CutsceneControl) -> Result<(), Q1Error> {
        match self.control_player.as_mut() {
            Some(control_player) => {
                control_player(actor, control);
                Ok(())
            }
            None => Err(q1_error("Q1 cinematic requires the selected player control host")),
        }
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! Scriptable in-crate mock host for Q1 unit tests.

    use std::collections::{HashMap, HashSet};

    use qa_core::identity::{IdentityOwner, SavedActorId};

    use super::*;

    /// In-memory actor registry with live/generation tracking.
    pub struct MockActors {
        /// Identity mint.
        pub identities: IdentityOwner,
        live: HashSet<(u32, u32)>,
        owners: HashMap<(u32, u32), OwnedActor>,
        sources: HashMap<(u32, u32), SourceSlot>,
        next_slot: u32,
    }

    impl MockActors {
        /// Fresh registry.
        pub fn new() -> Self {
            Self {
                identities: IdentityOwner::create("mock").expect("mock session"),
                live: HashSet::new(),
                owners: HashMap::new(),
                sources: HashMap::new(),
                next_slot: 1,
            }
        }

        fn key(actor: &ActorId) -> (u32, u32) {
            (actor.slot(), actor.generation())
        }
    }

    impl Default for MockActors {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q1SessionActorRegistry for MockActors {
        fn allocate_at_source(
            &mut self,
            owner: &ProviderId,
            source_slot: u32,
            definition: &str,
        ) -> Result<OwnedActor, Q1Error> {
            let _ = definition;
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.identities.actor(slot, 0);
            let owned = self
                .identities
                .owned_actor(&id, owner.clone())
                .map_err(|error| q1_error(error.to_string()))?;
            self.live.insert((slot, 0));
            self.owners.insert((slot, 0), owned.clone());
            self.sources.insert(
                (slot, 0),
                SourceSlot {
                    provider: owner.clone(),
                    slot: source_slot,
                },
            );
            Ok(owned)
        }

        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q1Error> {
            if self.live.contains(&Self::key(actor.id())) {
                Ok(())
            } else {
                Err(q1_error("Actor belongs to another session"))
            }
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            let key = Self::key(actor);
            if self.live.contains(&key) {
                self.owners.get(&key).cloned()
            } else {
                None
            }
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(&Self::key(actor))
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.owners
                .values()
                .filter(|owned| self.live.contains(&Self::key(owned.id())))
                .map(|owned| ActorObservation {
                    id: owned.id().clone(),
                    owner: owned.owner().clone(),
                    definition: String::from("mock:actor"),
                })
                .collect()
        }

        fn source_of(&self, actor: &ActorId) -> Option<SourceSlot> {
            self.sources.get(&Self::key(actor)).cloned()
        }

        fn release(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
            self.live.remove(&Self::key(actor.id()));
            Ok(())
        }

        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            let key = (saved.slot, saved.generation);
            if self.live.contains(&key) {
                self.owners.get(&key).cloned()
            } else {
                None
            }
        }

        fn reference_saved(&mut self, saved: &SavedActorId) -> ActorId {
            self.identities.actor(saved.slot, saved.generation)
        }
    }

    /// In-memory body table.
    #[derive(Default)]
    pub struct MockBodies {
        /// Stored bodies by slot.
        pub bodies: HashMap<(u32, u32), BodyState>,
    }

    impl Q1SharedBodyTable for MockBodies {
        fn create(&mut self, actor: &OwnedActor, initial: &BodyState) -> Result<(), Q1Error> {
            self.bodies
                .insert((actor.id().slot(), actor.id().generation()), initial.clone());
            Ok(())
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(&(actor.slot(), actor.generation())).cloned()
        }

        fn remove(&mut self, actor: &ActorId) {
            self.bodies.remove(&(actor.slot(), actor.generation()));
        }

        fn write(&mut self, actor: &OwnedActor, state: &BodyState) -> Result<(), Q1Error> {
            self.bodies
                .insert((actor.id().slot(), actor.id().generation()), state.clone());
            Ok(())
        }

        fn link(&mut self, _actor: &OwnedActor) -> Result<(), Q1Error> {
            Ok(())
        }

        fn linked(&self, actor: &ActorId) -> Option<super::super::gameplay::LinkedBody> {
            self.read(actor).map(|state| super::super::gameplay::LinkedBody {
                actor: actor.clone(),
                absolute_bounds: state.bounds,
                state,
                link_count: 1,
            })
        }

        fn attach(&mut self, _actor: &OwnedActor, _attachment: &BodyAttachment) -> Result<(), Q1Error> {
            Ok(())
        }

        fn detach(&mut self, _actor: &OwnedActor) -> Result<(), Q1Error> {
            Ok(())
        }
    }

    /// In-memory combat authority applying no policy.
    #[derive(Default)]
    pub struct MockCombat {
        /// Stored combat states by slot.
        pub states: HashMap<(u32, u32), CombatState>,
    }

    impl Q1GameplayAuthority for MockCombat {
        fn create(&mut self, actor: &OwnedActor, initial: &CombatState) -> Result<(), Q1Error> {
            self.states
                .insert((actor.id().slot(), actor.id().generation()), initial.clone());
            Ok(())
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.states.get(&(actor.slot(), actor.generation())).cloned()
        }

        fn set_health(&mut self, actor: &OwnedActor, health: f64) -> Result<(), Q1Error> {
            if !health.is_finite() {
                return Err(crate::q1::q1_range("Health must be finite"));
            }
            match self.states.get_mut(&(actor.id().slot(), actor.id().generation())) {
                Some(state) => {
                    state.health = health;
                    Ok(())
                }
                None => Err(q1_error("Actor has no combat binding")),
            }
        }

        fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) -> Result<(), Q1Error> {
            match self.states.get_mut(&(actor.id().slot(), actor.id().generation())) {
                Some(state) => {
                    state.armor = armor.clone();
                    Ok(())
                }
                None => Err(q1_error("Actor has no combat binding")),
            }
        }

        fn set_traits(&mut self, actor: &OwnedActor, traits: CombatTraits) -> Result<(), Q1Error> {
            match self.states.get_mut(&(actor.id().slot(), actor.id().generation())) {
                Some(state) => {
                    state.can_take_damage = traits.can_take_damage;
                    state.mass = traits.mass;
                    state.invulnerable = traits.invulnerable;
                    state.team = traits.team;
                    state.no_knockback = traits.no_knockback;
                    Ok(())
                }
                None => Err(q1_error("Actor has no combat binding")),
            }
        }

        fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState) -> Result<(), Q1Error> {
            let armor = self.read(actor.id()).map(|mut state| {
                state.armor.regular = regular.clone();
                state.armor
            });
            match armor {
                Some(armor) => self.set_armor(actor, &armor),
                None => Err(q1_error("Actor has no combat binding")),
            }
        }

        fn set_regular_points(&mut self, actor: &OwnedActor, points: f64) -> Result<(), Q1Error> {
            if !points.is_finite() {
                return Err(crate::q1::q1_range("Armor points must be finite"));
            }
            let armor = self.read(actor.id()).map(|mut state| {
                match &mut state.armor.regular {
                    RegularArmorState::Q1 { points: slot, .. }
                    | RegularArmorState::Q2 { points: slot, .. }
                    | RegularArmorState::Q3 { points: slot, .. }
                    | RegularArmorState::Source { points: slot, .. } => *slot = points,
                    RegularArmorState::None => {}
                }
                state.armor
            });
            match armor {
                Some(armor) => self.set_armor(actor, &armor),
                None => Err(q1_error("Actor has no combat binding")),
            }
        }

        fn bind_damage_adjustment(&mut self, _actor: &OwnedActor, _adjust: Q1DamageAdjustHook) {}

        fn apply(&mut self, request: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget {
                request: request.clone(),
            }
        }
    }

    /// In-memory inventory table with donor give/consume semantics.
    #[derive(Default)]
    pub struct MockInventory {
        /// Stored entries by slot.
        pub entries: HashMap<(u32, u32), Vec<InventoryEntry>>,
    }

    impl Q1SharedInventoryTable for MockInventory {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) -> Result<(), Q1Error> {
            let mut seen = HashSet::new();
            for entry in entries {
                if !seen.insert(entry.item.clone()) {
                    return Err(crate::q1::q1_range(format!("Duplicate inventory item {}", entry.item)));
                }
            }
            self.entries
                .insert((actor.id().slot(), actor.id().generation()), entries.to_vec());
            Ok(())
        }

        fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.entries
                .get(&(actor.slot(), actor.generation()))
                .cloned()
                .unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.entries.contains_key(&(actor.slot(), actor.generation()))
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.entries
                .get(&(actor.slot(), actor.generation()))
                .and_then(|entries| entries.iter().find(|entry| &entry.item == item))
                .map(|entry| entry.count)
                .unwrap_or(0.0)
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            let key = (actor.id().slot(), actor.id().generation());
            let Some(entries) = self.entries.get_mut(&key) else {
                return count == 0.0;
            };
            let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
                return count == 0.0;
            };
            if entry.count < count {
                return false;
            }
            entry.count -= count;
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let key = (actor.id().slot(), actor.id().generation());
            let Some(entries) = self.entries.get_mut(&key) else {
                return 0.0;
            };
            let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
                return 0.0;
            };
            let room = (entry.capacity - entry.count).max(0.0);
            let given = count.min(room);
            entry.count += given;
            given
        }

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) -> Result<(), Q1Error> {
            let key = (actor.id().slot(), actor.id().generation());
            let Some(entries) = self.entries.get_mut(&key) else {
                return Err(q1_error("Actor has no inventory binding"));
            };
            match entries.iter_mut().find(|candidate| candidate.item == entry.item) {
                Some(slot) => *slot = entry.clone(),
                None => entries.push(entry.clone()),
            }
            Ok(())
        }

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> Result<f64, Q1Error> {
            let key = (actor.id().slot(), actor.id().generation());
            let Some(entries) = self.entries.get_mut(&key) else {
                return Err(q1_error("Item is not a signed source counter"));
            };
            let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
                return Err(q1_error("Item is not a signed source counter"));
            };
            entry.count += delta;
            Ok(entry.count)
        }
    }

    /// In-memory callback bindings.
    #[derive(Default)]
    pub struct MockCallbacks {
        bound: HashSet<(u32, u32)>,
    }

    impl Q1ActorCallbackTable for MockCallbacks {
        fn bind(&mut self, actor: &OwnedActor) {
            self.bound.insert((actor.id().slot(), actor.id().generation()));
        }

        fn unbind(&mut self, actor: &OwnedActor) {
            self.bound.remove(&(actor.id().slot(), actor.id().generation()));
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.contains(&(actor.slot(), actor.generation()))
        }
    }

    /// Collected presentation events.
    #[derive(Default)]
    pub struct MockEvents {
        /// Emitted events in order.
        pub events: Vec<Q1Event>,
    }

    /// Build a scriptable mock host. Traces never hit; every think is
    /// recorded through the returned event log.
    pub fn mock_host() -> (Q1FoundationHost, std::rc::Rc<std::cell::RefCell<MockEvents>>) {
        let events = std::rc::Rc::new(std::cell::RefCell::new(MockEvents::default()));
        let emit_events = std::rc::Rc::clone(&events);
        let host = Q1FoundationHost {
            actors: Box::new(MockActors::new()),
            bodies: Box::new(MockBodies::default()),
            callbacks: Box::new(MockCallbacks::default()),
            combat: Box::new(MockCombat::default()),
            inventory: Box::new(MockInventory::default()),
            original_pickups: None,
            punch_angles: None,
            weapon_behavior: None,
            register_entity: None,
            source_damage_modifier: None,
            source_damage_powerup_owner: None,
            random: Box::new(|| 0.5),
            trace: Box::new(|request: &Q1TraceRequest| Q1Trace {
                fraction: 1.0,
                end: request.end,
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                actor: None,
                start_solid: false,
                all_solid: false,
                sky: false,
                in_open: true,
                in_water: false,
            }),
            contents: Box::new(|_| Q1Contents::Empty),
            walk_move: Box::new(|_, _, _| true),
            change_yaw: Box::new(|_| {}),
            move_to_goal: Box::new(|_, _, _, _| {}),
            check_bottom: Box::new(|_| true),
            schedule_think: Box::new(|_, _| {}),
            cancel_think: Box::new(|_| {}),
            emit: Box::new(move |event| emit_events.borrow_mut().events.push(event)),
            transition: Box::new(|_| {}),
            players: Box::new(Vec::new),
            check_client: Box::new(|_| None),
            classname: Box::new(|_| String::from("mock")),
            powerup: Box::new(|_, _, _| {}),
            step_pusher: Box::new(|actor, _| Q1PusherStep {
                actor: actor.clone(),
                status: Q1PusherStatus::Moved,
                moved: Vec::new(),
            }),
            weapon_impact: None,
            weapon_volume: None,
            monster_target: None,
            set_gravity: None,
            control_player: None,
            source_target: None,
            powerup_expires: None,
            source_damage_multiplier: None,
        };
        (host, events)
    }
}
