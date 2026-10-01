//! Q2 hand-grenade equipment runtime: per-actor input edges over the shared grenade controller.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/equipment-runtime.ts`
//! (`HandGrenadeRuntime`, `HandGrenadeRuntimeCheckpoint`, `HandGrenadeTravel`).
//!
//! Adaptations from the donor object graph:
//! - `Q2HandGrenadeEquipment` is a stateless handle in
//!   `qa_content::q2::equipment::hand_grenades`, so the session passes its
//!   `Q2GameServices` arena on every call instead of the runtime holding a
//!   `Q2EntityServices` reference. The donor aliases its `independent.game`
//!   to that same arena (`runtime.ts` passes one `game` for both), so no
//!   second arena exists here either; the session `SourceRandom` is likewise
//!   borrowed per call.
//! - The donor subscribes to actor release through `onRelease`, but the Rust
//!   arena cannot carry a self-borrowing subscription (see `Q2ActorRegistry`),
//!   so the session calls [`HandGrenadeRuntime::on_actor_released`] alongside
//!   `Q2GameServices::on_actor_released`.
//! - The runtime checkpoint mixes content and persistence records: the grenade
//!   controller converts inline (six action variants), while the foundation
//!   entities stay in the persistence shape behind
//!   [`Q2FoundationCheckpointBridge`] because that converter belongs to the Q2
//!   session partition.

use std::collections::HashMap;

use qa_content::contract::{HandGrenadeSelection, InventoryEntry};
use qa_content::q2::equipment::hand_grenades::{
    HandGrenadeCheckpoint, HandGrenadeEquipmentInput, HandGrenadeEquipmentState, HandGrenadeLoadout,
    Q2HandGrenadeEquipment, HAND_GRENADE_AMMO,
};
use qa_content::q2::foundation::host::Q2GameServices;
use qa_content::q2::foundation::weapons::hand_action::{HandAction, HandLifecycle};
use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;
use qa_world::WorldError;
use thiserror::Error;

use super::random::{RandomCheckpoint, SourceRandom};
use crate::persistence::q2::foundation::Q2FoundationCheckpoint as Q2FoundationSave;

/// Checkpoint version for [`HandGrenadeRuntimeCheckpoint`].
pub(crate) const CHECKPOINT_VERSION: u8 = 1;

/// Latched input edges for one admitted actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HandControl {
    held: bool,
    pressed: bool,
    released: bool,
}

impl HandControl {
    fn idle() -> Self {
        Self {
            held: false,
            pressed: false,
            released: false,
        }
    }
}

/// One saved input-edge record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandControlEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Held flag.
    pub held: bool,
    /// Latched press.
    pub pressed: bool,
    /// Latched release.
    pub released: bool,
}

/// Source continuation half of the checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeSourceCheckpoint {
    /// Foundation entities in the persistence shape.
    pub entities: Q2FoundationSave,
    /// Source RNG stream.
    pub random: RandomCheckpoint,
}

/// Hand-grenade runtime checkpoint (`HandGrenadeRuntimeCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeRuntimeCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u8,
    /// Grenade controller state.
    pub controller: HandGrenadeCheckpoint,
    /// Per-actor input edges.
    pub controls: Vec<HandControlEntry>,
    /// Source continuations.
    pub source: HandGrenadeSourceCheckpoint,
}

/// Cross-map carry for one actor (`HandGrenadeTravel`).
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeTravel {
    /// Equipment state at capture.
    pub state: HandGrenadeEquipmentState,
    /// Canonical ammunition entry at capture.
    pub ammo: InventoryEntry,
    /// Session seconds at capture.
    pub seconds: f64,
}

/// Frame input without the edge flags (`HandGrenadeEquipmentInput` minus
/// `pressed`/`held`/`released`, which the runtime latches itself).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandGrenadeRuntimeInput {
    /// Lifecycle.
    pub lifecycle: HandLifecycle,
    /// Aim angles.
    pub angles: Vec3,
    /// Gravity.
    pub gravity: f64,
    /// Quad damage until.
    pub quad_until: f64,
    /// Double damage until.
    pub double_until: f64,
    /// Quad-fire until.
    pub quad_fire_until: f64,
    /// Haste.
    pub haste: bool,
    /// No stacked double.
    pub no_stack_double: bool,
    /// Whether players collide.
    pub players_collide: bool,
}

/// Value seam: persistence-shaped foundation entities in and out of the live
/// arena (donor `Q2EntityServices.capture`/`restore` in
/// `src/content/q2/foundation/entity-services.ts` over the
/// `src/persistence/q2-foundation.ts` record; canonical home: the Q2 session
/// partition); unify post-merge.
pub trait Q2FoundationCheckpointBridge {
    /// Capture the arena entities into the persistence shape.
    fn capture_entities(&self, game: &Q2GameServices) -> Q2FoundationSave;
    /// Restore a persisted entities snapshot into the arena.
    fn restore_entities(&self, game: &mut Q2GameServices, checkpoint: &Q2FoundationSave) -> Result<(), WorldError>;
}

/// Hand-grenade equipment failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EquipmentError {
    /// Selection disables hand grenades.
    #[error("Hand grenade equipment is disabled")]
    Disabled,
    /// Actor has no live owner.
    #[error("Equipment travel requires a live actor")]
    UnknownOwner,
    /// Actor was never admitted.
    #[error("Actor has no selected hand grenade equipment")]
    MissingControls,
    /// Shared inventory lost the canonical ammunition.
    #[error("Equipment travel lost its canonical ammunition")]
    MissingAmmunition,
    /// Checkpoint edition differs from the arena.
    #[error("Hand grenade checkpoint edition mismatch")]
    EditionMismatch,
    /// Saved input owner is missing, stale, or duplicated.
    #[error("Invalid equipment input owner")]
    InvalidControlsOwner,
    /// Saved inputs do not cover every controller actor.
    #[error("Equipment checkpoint is missing an action's input state")]
    MissingControlsState,
    /// Source RNG restore failed.
    #[error("Hand grenade random restore failed")]
    Random,
    /// Foundation entities restore failed.
    #[error("Hand grenade entities restore failed: {0}")]
    Foundation(String),
}

/// Shift an action's clock by a session-time delta.
fn rebase(action: HandAction, delta: f64) -> HandAction {
    match action {
        HandAction::Idle | HandAction::Disarmed => action,
        HandAction::Preparing {
            frame,
            next_at,
            release_queued,
        } => HandAction::Preparing {
            frame,
            next_at: next_at + delta,
            release_queued,
        },
        HandAction::Cooking { expires_at } => HandAction::Cooking {
            expires_at: expires_at + delta,
        },
        HandAction::Releasing { expires_at, throw_at } => HandAction::Releasing {
            expires_at: expires_at + delta,
            throw_at: throw_at + delta,
        },
        HandAction::Recovering {
            ready_at,
            require_release,
        } => HandAction::Recovering {
            ready_at: ready_at + delta,
            require_release,
        },
    }
}

/// Q2 hand-grenade equipment runtime (`HandGrenadeRuntime`).
///
/// Action and input edges share the session's actors, ammunition, and source
/// continuations; the arena and RNG are borrowed per call.
pub struct HandGrenadeRuntime {
    selection: HandGrenadeSelection,
    controller: Q2HandGrenadeEquipment,
    bridge: Box<dyn Q2FoundationCheckpointBridge>,
    controls: HashMap<ActorId, HandControl>,
}

impl HandGrenadeRuntime {
    /// Create a runtime over an enabled selection.
    pub fn new(
        selection: HandGrenadeSelection,
        controller: Q2HandGrenadeEquipment,
        bridge: Box<dyn Q2FoundationCheckpointBridge>,
    ) -> Result<Self, EquipmentError> {
        if !matches!(selection, HandGrenadeSelection::Enabled { .. }) {
            return Err(EquipmentError::Disabled);
        }
        Ok(Self {
            selection,
            controller,
            bridge,
            controls: HashMap::new(),
        })
    }

    /// The validated selection.
    #[must_use]
    pub fn selection(&self) -> &HandGrenadeSelection {
        &self.selection
    }

    /// The shared grenade controller.
    #[must_use]
    pub fn controller(&self) -> Q2HandGrenadeEquipment {
        self.controller
    }

    fn allowance(&self) -> (f64, f64) {
        let HandGrenadeSelection::Enabled {
            initial_ammo, capacity, ..
        } = &self.selection
        else {
            unreachable!("HandGrenadeRuntime::new rejects disabled selections");
        };
        (*initial_ammo, *capacity)
    }

    /// Install one actor's action through a capture round trip.
    fn install_action(
        controller: Q2HandGrenadeEquipment,
        game: &mut Q2GameServices,
        actor: &ActorId,
        action: HandAction,
    ) {
        let mut checkpoint = controller.capture(game);
        for entry in checkpoint.actors.values_mut() {
            if entry.actor.slot == actor.slot() && entry.actor.generation == actor.generation() {
                entry.action = action;
            }
        }
        controller.restore(game, checkpoint);
    }

    /// Admit an actor, optionally carrying travel state.
    pub fn admit(
        &mut self,
        actor: &ActorId,
        game: &mut Q2GameServices,
        carry: Option<&HandGrenadeTravel>,
    ) -> Result<(), EquipmentError> {
        let controller = self.controller;
        let first = controller.state_snapshot(game, actor.clone()).is_none();
        let owner = game
            .host
            .actors()
            .resolve_owned(actor)
            .ok_or(EquipmentError::UnknownOwner)?;
        let (initial_ammo, capacity) = self.allowance();
        controller.configure(
            &owner,
            game,
            HandGrenadeLoadout {
                enabled: true,
                initial_ammo,
                capacity,
                infinite_ammo: false,
            },
        );
        self.controls.insert(actor.clone(), HandControl::idle());
        if first && carry.is_none() {
            self.grant_initial(actor, game)?;
        }
        if let Some(carry) = carry {
            let owner = game
                .host
                .actors()
                .resolve_owned(actor)
                .ok_or(EquipmentError::UnknownOwner)?;
            game.host.inventory().configure(&owner, &carry.ammo);
            let delta = game.host.now() - carry.seconds;
            Self::install_action(controller, game, actor, rebase(carry.state.action, delta));
        }
        Ok(())
    }

    fn grant_initial(&self, actor: &ActorId, game: &mut Q2GameServices) -> Result<(), EquipmentError> {
        let owner = game
            .host
            .actors()
            .resolve_owned(actor)
            .ok_or(EquipmentError::UnknownOwner)?;
        let ammo = game
            .host
            .inventory()
            .entries(actor)
            .into_iter()
            .find(|entry| entry.item == HAND_GRENADE_AMMO)
            .ok_or(EquipmentError::MissingAmmunition)?;
        let (initial_ammo, capacity) = self.allowance();
        game.host.inventory().configure(
            &owner,
            &InventoryEntry {
                item: ammo.item.clone(),
                count: ammo.count.max(initial_ammo),
                capacity: ammo.capacity.max(capacity),
                count_policy: ammo.count_policy,
            },
        );
        Ok(())
    }

    /// Respawn an actor with a fresh allowance and an idle hand.
    pub fn respawn(&mut self, actor: &ActorId, game: &mut Q2GameServices) -> Result<(), EquipmentError> {
        self.admit(actor, game, None)?;
        self.grant_initial(actor, game)?;
        Self::install_action(self.controller, game, actor, HandAction::Idle);
        Ok(())
    }

    /// Latch the held flag and its edges.
    pub fn input(&mut self, actor: &ActorId, held: bool) -> Result<(), EquipmentError> {
        let previous = self
            .controls
            .get(actor)
            .copied()
            .ok_or(EquipmentError::MissingControls)?;
        self.controls.insert(
            actor.clone(),
            HandControl {
                held,
                pressed: previous.pressed || (held && !previous.held),
                released: previous.released || (!held && previous.held),
            },
        );
        Ok(())
    }

    /// Step one actor's hand over the shared controller.
    pub fn step(
        &mut self,
        actor: &ActorId,
        game: &mut Q2GameServices,
        input: &HandGrenadeRuntimeInput,
        project: &mut dyn FnMut(Vec3, Vec3) -> (Vec3, Vec3),
        enabled: bool,
    ) {
        let Some(controls) = self.controls.get(actor).copied() else {
            return;
        };
        let controller = self.controller;
        if let Some(state) = controller.state_snapshot(game, actor.clone()) {
            if state.config.enabled != enabled {
                if let Some(owner) = game.host.actors().resolve_owned(actor) {
                    controller.configure(
                        &owner,
                        game,
                        HandGrenadeLoadout {
                            enabled,
                            ..state.config
                        },
                    );
                }
            }
        }
        self.controls.insert(
            actor.clone(),
            HandControl {
                held: controls.held,
                pressed: false,
                released: false,
            },
        );
        controller.step(
            actor.clone(),
            game,
            &HandGrenadeEquipmentInput {
                pressed: controls.pressed,
                held: controls.held,
                released: controls.released,
                lifecycle: input.lifecycle,
                angles: input.angles,
                gravity: input.gravity,
                quad_until: input.quad_until,
                double_until: input.double_until,
                quad_fire_until: input.quad_fire_until,
                haste: input.haste,
                no_stack_double: input.no_stack_double,
                players_collide: input.players_collide,
            },
            project,
        );
    }

    /// Capture cross-map carry for one actor.
    ///
    /// The arena borrows mutably only because the host table accessors require
    /// it; travel itself writes nothing.
    pub fn travel(
        &self,
        actor: &ActorId,
        game: &mut Q2GameServices,
    ) -> Result<Option<HandGrenadeTravel>, EquipmentError> {
        let Some(state) = self.controller.state_snapshot(game, actor.clone()) else {
            return Ok(None);
        };
        let ammo = game
            .host
            .inventory()
            .entries(actor)
            .into_iter()
            .find(|entry| entry.item == HAND_GRENADE_AMMO)
            .ok_or(EquipmentError::MissingAmmunition)?;
        Ok(Some(HandGrenadeTravel {
            state,
            ammo,
            seconds: game.host.now(),
        }))
    }

    /// Capture the runtime checkpoint.
    pub fn capture(&self, game: &Q2GameServices, random: &SourceRandom) -> HandGrenadeRuntimeCheckpoint {
        let mut controls: Vec<HandControlEntry> = self
            .controls
            .iter()
            .map(|(actor, control)| HandControlEntry {
                actor: SavedActorId::from(actor),
                held: control.held,
                pressed: control.pressed,
                released: control.released,
            })
            .collect();
        controls.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
        HandGrenadeRuntimeCheckpoint {
            version: CHECKPOINT_VERSION,
            controller: self.controller.capture(game),
            controls,
            source: HandGrenadeSourceCheckpoint {
                entities: self.bridge.capture_entities(game),
                random: random.checkpoint(),
            },
        }
    }

    /// Restore a checkpoint over the arena.
    pub fn restore(
        &mut self,
        game: &mut Q2GameServices,
        random: &mut SourceRandom,
        checkpoint: HandGrenadeRuntimeCheckpoint,
    ) -> Result<(), EquipmentError> {
        random
            .restore(&checkpoint.source.random)
            .map_err(|_| EquipmentError::Random)?;
        self.bridge
            .restore_entities(game, &checkpoint.source.entities)
            .map_err(|error| EquipmentError::Foundation(error.to_string()))?;
        if checkpoint.controller.edition != game.options.edition {
            return Err(EquipmentError::EditionMismatch);
        }
        let controller = self.controller;
        let expected = checkpoint.controller.actors.len();
        controller.restore(game, checkpoint.controller);
        self.controls.clear();
        for entry in &checkpoint.controls {
            let Some(owner) = game.host.actors().resolve_saved(entry.actor) else {
                return Err(EquipmentError::InvalidControlsOwner);
            };
            if controller.state_snapshot(game, owner.id().clone()).is_none() || self.controls.contains_key(owner.id()) {
                return Err(EquipmentError::InvalidControlsOwner);
            }
            self.controls.insert(
                owner.id().clone(),
                HandControl {
                    held: entry.held,
                    pressed: entry.pressed,
                    released: entry.released,
                },
            );
        }
        if self.controls.len() != expected {
            return Err(EquipmentError::MissingControlsState);
        }
        Ok(())
    }

    /// Drop an actor's latched input. The session calls this alongside
    /// `Q2GameServices::on_actor_released`; the arena cannot carry the donor's
    /// `onRelease` subscription itself.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.controls.remove(actor);
    }
}

#[cfg(test)]
pub mod fakes {
    //! Fake Q2 arena for the equipment and grapple tests.
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use qa_content::contract::{ArmorState, InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState};
    use qa_content::q2::foundation::host::{
        Q2Edition, Q2FoundationHost, Q2GameOptions, Q2GameServices, Q2LandmarkCarry, Q2Mode, Q2Motion,
        Q2PlayerViewState, Q2PresentationEvent, Q2Solid, Q2TraceRequest,
    };
    use qa_content::q2::support::contracts::{
        ActorObservation, BodyAttachment, BodyState, CombatState, CombatTraitChanges, DamageOutcome, DamageRequest,
        LinkedBody, PowerArmorCells, Q2BspPlane, Q2TraceFields, TraceContact, TraceFamily, TraceHit, TraceResult,
        TransitionIntent,
    };
    use qa_content::q2::support::tables::{
        Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
    };
    use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId};
    use qa_core::math::{vec3, Bounds, Vec3};

    use super::*;

    /// Minting fake Q2 host with real actor, body, inventory, and clock state.
    pub struct FakeQ2Host {
        identities: IdentityOwner,
        provider: ProviderId,
        live: HashSet<ActorId>,
        owned: HashMap<ActorId, OwnedActor>,
        next_slot: u32,
        bodies: HashMap<ActorId, BodyState>,
        attachments: HashMap<ActorId, BodyAttachment>,
        bound: HashSet<ActorId>,
        inventory: HashMap<ActorId, Vec<InventoryEntry>>,
        now: Rc<RefCell<f64>>,
    }

    impl FakeQ2Host {
        /// Fresh host at session time 100.
        pub fn new() -> Self {
            Self {
                identities: IdentityOwner::create("equipment-test").expect("owner"),
                provider: ProviderId::new("q2", "test"),
                live: HashSet::new(),
                owned: HashMap::new(),
                next_slot: 1,
                bodies: HashMap::new(),
                attachments: HashMap::new(),
                bound: HashSet::new(),
                inventory: HashMap::new(),
                now: Rc::new(RefCell::new(100.0)),
            }
        }

        /// Shared session clock.
        pub fn clock(&self) -> Rc<RefCell<f64>> {
            self.now.clone()
        }

        fn mint_inner(&mut self, owner: &ProviderId) -> OwnedActor {
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.identities.actor(slot, 0);
            let owned = self.identities.owned_actor(&id, owner.clone()).expect("fake owner");
            self.live.insert(id.clone());
            self.owned.insert(id, owned.clone());
            owned
        }

        fn is_live_inner(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
    }

    impl Default for FakeQ2Host {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q2ActorRegistry for FakeQ2Host {
        fn allocate(&mut self, owner: &ProviderId, _definition: &str) -> OwnedActor {
            self.mint_inner(owner)
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, _source_slot: u32, _definition: &str) -> OwnedActor {
            self.mint_inner(owner)
        }

        fn source_of(&self, _actor: &ActorId) -> Option<(ProviderId, u32)> {
            None
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.live.remove(actor.id());
        }

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

        fn observations(&self) -> Vec<ActorObservation> {
            self.owned
                .values()
                .filter(|owned| self.is_live_inner(owned.id()))
                .map(|owned| ActorObservation {
                    id: owned.id().clone(),
                    owner: self.provider.clone(),
                    definition: "q2:test".to_string(),
                })
                .collect()
        }

        fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            self.owned
                .values()
                .find(|owned| {
                    owned.id().slot() == saved.slot
                        && owned.id().generation() == saved.generation
                        && self.is_live_inner(owned.id())
                })
                .cloned()
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.identities.actor(saved.slot, saved.generation)
        }

        fn assert_owned(&self, actor: &OwnedActor) {
            assert!(self.is_live_inner(actor.id()), "fake actor is live");
        }
    }

    impl Q2BodyTable for FakeQ2Host {
        fn create(&mut self, actor: &OwnedActor, initial: &BodyState) {
            self.bodies.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &BodyState) {
            self.bodies.insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) {
            self.attachments.insert(actor.id().clone(), attachment.clone());
        }

        fn detach(&mut self, actor: &OwnedActor) {
            self.attachments.remove(actor.id());
        }

        fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment> {
            self.attachments.get(actor).cloned()
        }

        fn linked(&self, _actor: &ActorId) -> Option<LinkedBody> {
            None
        }

        fn link(&mut self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&mut self, _actor: &OwnedActor) {}
    }

    impl Q2CallbackTable for FakeQ2Host {
        fn bind(&mut self, actor: &OwnedActor) {
            self.bound.insert(actor.id().clone());
        }

        fn unbind(&mut self, actor: &ActorId) {
            self.bound.remove(actor);
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.contains(actor)
        }

        fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
    }

    impl Q2CombatAuthority for FakeQ2Host {
        fn create(&mut self, _actor: &OwnedActor, _initial: &CombatState) {}

        fn read(&self, _actor: &ActorId) -> Option<CombatState> {
            None
        }

        fn set_health(&mut self, _actor: &OwnedActor, _health: f64) {}

        fn set_armor(&mut self, _actor: &OwnedActor, _armor: &ArmorState) {}

        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64, _initial: Option<&RegularArmorState>) {}

        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &RegularArmorState) {}

        fn set_powered_protection(&mut self, _actor: &OwnedActor, _powered: &PoweredProtectionState) {}

        fn set_traits(&mut self, _actor: &OwnedActor, _changes: &CombatTraitChanges) {}

        fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    impl Q2InventoryTable for FakeQ2Host {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) {
            self.inventory
                .entry(actor.id().clone())
                .or_default()
                .extend(entries.iter().cloned());
        }

        fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.inventory.get(actor).cloned().unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.inventory.get(actor).is_some_and(|entries| !entries.is_empty())
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.inventory
                .get(actor)
                .and_then(|entries| entries.iter().find(|entry| &entry.item == item))
                .map_or(0.0, |entry| entry.count)
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            let Some(entry) = self
                .inventory
                .get_mut(actor.id())
                .and_then(|entries| entries.iter_mut().find(|entry| &entry.item == item))
            else {
                return false;
            };
            if entry.count < count {
                return false;
            }
            entry.count -= count;
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let entries = self.inventory.entry(actor.id().clone()).or_default();
            if let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) {
                entry.count += count;
            } else {
                entries.push(InventoryEntry {
                    item: item.clone(),
                    count,
                    capacity: count,
                    count_policy: None,
                });
            }
            count
        }

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) {
            let entries = self.inventory.entry(actor.id().clone()).or_default();
            if let Some(slot) = entries.iter_mut().find(|slot| slot.item == entry.item) {
                *slot = entry.clone();
            } else {
                entries.push(entry.clone());
            }
        }

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64 {
            let Some(entry) = self
                .inventory
                .get_mut(actor.id())
                .and_then(|entries| entries.iter_mut().find(|entry| &entry.item == item))
            else {
                return 0.0;
            };
            entry.count += delta;
            entry.count
        }
    }

    impl Q2FoundationHost for FakeQ2Host {
        fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
            self
        }

        fn bodies(&mut self) -> &mut dyn Q2BodyTable {
            self
        }

        fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
            self
        }

        fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
            self
        }

        fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
            self
        }

        fn now(&self) -> f64 {
            *self.now.borrow()
        }

        fn frame_seconds(&self) -> f64 {
            0.1
        }

        fn gravity(&self) -> f64 {
            800.0
        }

        fn random(&mut self) -> f64 {
            0.5
        }

        fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn trace(&mut self, request: &Q2TraceRequest) -> TraceResult {
            // Empty world: every trace completes without contact.
            TraceResult {
                fraction: 1.0,
                end: request.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                family: TraceFamily::Q2(Q2TraceFields {
                    contents: 0,
                    surface: None,
                    source_plane: Q2BspPlane {
                        normal: vec3(0.0, 0.0, 1.0),
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    secondary: None,
                }),
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            false
        }

        fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            false
        }

        fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
            false
        }

        fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
            Vec::new()
        }

        fn players(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn world_actor(&mut self) -> ActorId {
            self.identities.actor(0, 0)
        }

        fn is_player(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn is_monster(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
            Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            }
        }

        fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

        fn set_motion(&mut self, _motion: &Q2Motion) {}

        fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

        fn emit(&mut self, _event: Q2PresentationEvent) {}

        fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
            None
        }

        fn key_consumed(&mut self, _player: &ActorId) {}

        fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

        fn transition(&mut self, _intent: TransitionIntent) {}

        fn diagnostic(&mut self, _message: &str) {}
    }

    /// Default fake body.
    pub fn test_body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            ground: None,
        }
    }

    /// Classic singleplayer game options for tests.
    pub fn test_options() -> Q2GameOptions {
        Q2GameOptions {
            edition: Q2Edition::Classic,
            map_name: "test".to_string(),
            skill: 1,
            mode: Q2Mode::Singleplayer,
            deathmatch_flags: 0,
            max_clients: 1,
            provider: ProviderId::new("q2", "test"),
            damage_powerup_owner: None,
            source_damage_modifier: None,
            campaign: ProviderId::new("q2", "test"),
            combat_provider: ProviderId::new("q2", "test"),
            inventory_provider: ProviderId::new("q2", "test"),
            movement_provider: ProviderId::new("q2", "test"),
        }
    }

    /// Build a game over a fresh fake host, returning the shared clock.
    pub fn test_game() -> (Q2GameServices, Rc<RefCell<f64>>) {
        let host = FakeQ2Host::new();
        let clock = host.clock();
        let game = Q2GameServices::new(Box::new(host), test_options(), Vec::new());
        (game, clock)
    }

    /// Recording foundation bridge over inert snapshots.
    pub struct FakeBridge {
        /// Restored snapshots, in order.
        pub restored: RefCell<Vec<Q2FoundationSave>>,
    }

    impl FakeBridge {
        /// Fresh bridge.
        pub fn new() -> Self {
            Self {
                restored: RefCell::new(Vec::new()),
            }
        }
    }

    impl Default for FakeBridge {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Q2FoundationCheckpointBridge for FakeBridge {
        fn capture_entities(&self, _game: &Q2GameServices) -> Q2FoundationSave {
            Q2FoundationSave {
                next_source_slot: 0,
                sequence: 0,
                freed_slots: Vec::new(),
                counters: crate::persistence::q2::foundation::Q2FoundationCounters {
                    total_secrets: 0.0,
                    found_secrets: 0.0,
                    total_goals: 0.0,
                    found_goals: 0.0,
                    total_monsters: 0.0,
                    killed_monsters: 0.0,
                    server_flags: 0.0,
                },
                entities: Vec::new(),
            }
        }

        fn restore_entities(
            &self,
            _game: &mut Q2GameServices,
            checkpoint: &Q2FoundationSave,
        ) -> Result<(), WorldError> {
            self.restored.borrow_mut().push(checkpoint.clone());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_content::contract::{ContentId, ProviderReference, SourceEdition};
    use qa_core::identity::{OwnedActor, ProviderId};
    use qa_core::math::vec3;

    use super::fakes::*;
    use super::*;

    fn selection() -> HandGrenadeSelection {
        HandGrenadeSelection::Enabled {
            source: ProviderReference {
                provider: ProviderId::new("q2", "test"),
                content: ContentId("q2:test:test:1".to_string()),
            },
            edition: SourceEdition::Classic,
            initial_ammo: 5.0,
            capacity: 50.0,
        }
    }

    struct Rig {
        game: Q2GameServices,
        clock: Rc<RefCell<f64>>,
        runtime: HandGrenadeRuntime,
    }

    fn rig() -> Rig {
        let (mut game, clock) = test_game();
        let controller = Q2HandGrenadeEquipment::new(&mut game);
        let runtime = HandGrenadeRuntime::new(selection(), controller, Box::new(FakeBridge::new())).expect("runtime");
        Rig { game, clock, runtime }
    }

    fn admit_actor(rig: &mut Rig) -> OwnedActor {
        let owned = rig
            .game
            .host
            .actors()
            .allocate(&ProviderId::new("q2", "test"), "q2:test");
        rig.game.host.bodies().create(&owned, &test_body());
        owned
    }

    fn frame_input() -> HandGrenadeRuntimeInput {
        HandGrenadeRuntimeInput {
            lifecycle: HandLifecycle::Alive,
            angles: vec3(0.0, 0.0, 0.0),
            gravity: 800.0,
            quad_until: 0.0,
            double_until: 0.0,
            quad_fire_until: 0.0,
            haste: false,
            no_stack_double: false,
            players_collide: false,
        }
    }

    fn project() -> impl FnMut(Vec3, Vec3) -> (Vec3, Vec3) {
        |from, _direction| (from, from)
    }

    fn ammo_count(rig: &mut Rig, actor: &ActorId) -> (f64, f64) {
        let entry = rig
            .game
            .host
            .inventory()
            .entries(actor)
            .into_iter()
            .find(|entry| entry.item == HAND_GRENADE_AMMO)
            .expect("ammo entry");
        (entry.count, entry.capacity)
    }

    #[test]
    fn rejects_disabled_selection() {
        let (mut game, _) = test_game();
        let controller = Q2HandGrenadeEquipment::new(&mut game);
        assert!(matches!(
            HandGrenadeRuntime::new(HandGrenadeSelection::Disabled, controller, Box::new(FakeBridge::new())),
            Err(EquipmentError::Disabled)
        ));
    }

    #[test]
    fn admits_with_initial_allowance() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        let state = rig
            .runtime
            .controller()
            .state_snapshot(&rig.game, owned.id().clone())
            .expect("state");
        assert!(state.config.enabled);
        assert_eq!(state.config.initial_ammo, 5.0);
        assert_eq!(state.config.capacity, 50.0);
        assert_eq!(state.action, HandAction::Idle);
        assert_eq!(ammo_count(&mut rig, owned.id()), (5.0, 50.0));
    }

    #[test]
    fn input_latches_edges_until_step() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        rig.runtime.input(owned.id(), true).expect("press");
        let random = SourceRandom::new(1);
        let checkpoint = rig.runtime.capture(&rig.game, &random);
        assert_eq!(checkpoint.controls.len(), 1);
        assert!(checkpoint.controls[0].pressed);
        rig.runtime
            .step(owned.id(), &mut rig.game, &frame_input(), &mut project(), true);
        let checkpoint = rig.runtime.capture(&rig.game, &random);
        assert!(!checkpoint.controls[0].pressed);
        assert!(checkpoint.controls[0].held);
        let state = rig
            .runtime
            .controller()
            .state_snapshot(&rig.game, owned.id().clone())
            .expect("state");
        assert!(matches!(state.action, HandAction::Preparing { .. }));
    }

    #[test]
    fn travel_rebases_action_clock() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        rig.runtime.input(owned.id(), true).expect("press");
        rig.runtime
            .step(owned.id(), &mut rig.game, &frame_input(), &mut project(), true);
        let travel = rig
            .runtime
            .travel(owned.id(), &mut rig.game)
            .expect("travel")
            .expect("carry");
        assert_eq!(travel.seconds, 100.0);
        let HandAction::Preparing { next_at, .. } = travel.state.action else {
            panic!("expected a preparing hand, got {:?}", travel.state.action);
        };
        *rig.clock.borrow_mut() = 110.0;
        let owned2 = admit_actor(&mut rig);
        rig.runtime
            .admit(owned2.id(), &mut rig.game, Some(&travel))
            .expect("carry admit");
        let state = rig
            .runtime
            .controller()
            .state_snapshot(&rig.game, owned2.id().clone())
            .expect("state");
        let HandAction::Preparing { next_at: rebased, .. } = state.action else {
            panic!("expected a rebased preparing hand, got {:?}", state.action);
        };
        assert_eq!(rebased, next_at + 10.0);
        // The stepped throw consumed one grenade before travel; the carry
        // installs that ammunition verbatim.
        assert_eq!(travel.ammo.count, 4.0);
        assert_eq!(ammo_count(&mut rig, owned2.id()), (4.0, 50.0));
    }

    #[test]
    fn respawn_restores_idle_and_allowance() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        assert!(rig
            .game
            .host
            .inventory()
            .consume(&owned, &HAND_GRENADE_AMMO.to_string(), 3.0));
        rig.runtime.input(owned.id(), true).expect("press");
        rig.runtime
            .step(owned.id(), &mut rig.game, &frame_input(), &mut project(), true);
        rig.runtime.respawn(owned.id(), &mut rig.game).expect("respawn");
        let state = rig
            .runtime
            .controller()
            .state_snapshot(&rig.game, owned.id().clone())
            .expect("state");
        assert_eq!(state.action, HandAction::Idle);
        assert_eq!(ammo_count(&mut rig, owned.id()), (5.0, 50.0));
    }

    #[test]
    fn step_ignores_unadmitted_actor() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime
            .step(owned.id(), &mut rig.game, &frame_input(), &mut project(), true);
        assert!(rig
            .runtime
            .controller()
            .state_snapshot(&rig.game, owned.id().clone())
            .is_none());
    }

    #[test]
    fn input_rejects_unadmitted_actor() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        assert_eq!(
            rig.runtime.input(owned.id(), true),
            Err(EquipmentError::MissingControls)
        );
    }

    #[test]
    fn admit_rejects_released_owner() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.game.host.actors().release(&owned);
        rig.runtime.on_actor_released(owned.id());
        assert_eq!(
            rig.runtime.admit(owned.id(), &mut rig.game, None),
            Err(EquipmentError::UnknownOwner)
        );
    }

    #[test]
    fn released_actor_drops_controls() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        rig.runtime.on_actor_released(owned.id());
        assert_eq!(
            rig.runtime.input(owned.id(), true),
            Err(EquipmentError::MissingControls)
        );
    }

    #[test]
    fn travel_without_state_is_empty() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        assert!(rig.runtime.travel(owned.id(), &mut rig.game).expect("travel").is_none());
    }

    #[test]
    fn capture_restore_round_trip() {
        let mut rig = rig();
        let first = admit_actor(&mut rig);
        let second = admit_actor(&mut rig);
        rig.runtime.admit(first.id(), &mut rig.game, None).expect("admit");
        rig.runtime.admit(second.id(), &mut rig.game, None).expect("admit");
        rig.runtime.input(first.id(), true).expect("press");
        let mut random = SourceRandom::new(7);
        let checkpoint = rig.runtime.capture(&rig.game, &random);
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.controller.actors.len(), 2);
        assert_eq!(checkpoint.controls.len(), 2);
        let expected = random.next_integer();
        random.next_integer();
        random.next_integer();
        // Restore over the live arena: actors, bodies, and ammo still resolve.
        let controller = rig.runtime.controller();
        let mut runtime =
            HandGrenadeRuntime::new(selection(), controller, Box::new(FakeBridge::new())).expect("runtime");
        runtime
            .restore(&mut rig.game, &mut random, checkpoint)
            .expect("restore");
        assert_eq!(random.next_integer(), expected);
        runtime.input(first.id(), false).expect("controls rebuilt");
        assert!(runtime.travel(first.id(), &mut rig.game).expect("travel").is_some());
    }

    #[test]
    fn restore_rejects_duplicate_controls() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        let mut random = SourceRandom::new(1);
        let mut checkpoint = rig.runtime.capture(&rig.game, &random);
        checkpoint.controls.push(checkpoint.controls[0].clone());
        assert_eq!(
            rig.runtime.restore(&mut rig.game, &mut random, checkpoint),
            Err(EquipmentError::InvalidControlsOwner)
        );
    }

    #[test]
    fn restore_rejects_missing_controls() {
        let mut rig = rig();
        let owned = admit_actor(&mut rig);
        rig.runtime.admit(owned.id(), &mut rig.game, None).expect("admit");
        let mut random = SourceRandom::new(1);
        let mut checkpoint = rig.runtime.capture(&rig.game, &random);
        checkpoint.controls.clear();
        assert_eq!(
            rig.runtime.restore(&mut rig.game, &mut random, checkpoint),
            Err(EquipmentError::MissingControlsState)
        );
    }
}
