//! Q2 engine-table boundaries.
//!
//! Donor provenance (shapes only; owned by other lanes):
//! `src/world/actors/registry.ts` (`SessionActorRegistry`),
//! `src/world/actors/body.ts` (`SharedBodyTable`),
//! `src/world/actors/callbacks.ts` (`ActorCallbackTable`),
//! `src/world/gameplay/authority.ts` (`GameplayAuthority`),
//! `src/world/gameplay/inventory.ts` (`SharedInventoryTable`).
//!
//! Only the methods Q2 content actually calls are modeled. The donor's
//! callback table binds closures; the Rust engine instead routes physics
//! dispatch through
//! [`Q2GameServices`](crate::q2::foundation::host::Q2GameServices)
//! dispatch methods, so this table keeps binding bookkeeping only.

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::{ArmorState, InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState};

use super::contracts::{
    ActorObservation, BodyAttachment, BodyState, CombatState, CombatTraitChanges, DamageOutcome, DamageRequest,
    LinkedBody, PowerArmorCells,
};

/// Session actor registry surface used by Q2 (`SessionActorRegistry`).
pub trait Q2ActorRegistry {
    /// Allocate an actor for an owner.
    fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor;
    /// Allocate an actor at a source slot.
    fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor;
    /// Source location of an actor.
    fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)>;
    /// Release an actor.
    fn release(&mut self, actor: &OwnedActor);
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an actor to its owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Live actor observations.
    fn observations(&self) -> Vec<ActorObservation>;
    /// Resolve a saved actor reference.
    fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
    /// Reference a saved actor in the checkpoint domain.
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Assert session ownership of an actor handle.
    fn assert_owned(&self, actor: &OwnedActor);
}

/// Release-notification duty for the engine implementor.
///
/// The donor registry pushes release notifications through `onRelease`
/// subscriptions; the arena owns the host, so a self-borrowing
/// subscription cannot exist. Instead the engine calls
/// [`Q2GameServices::on_actor_released`](crate::q2::foundation::host::Q2GameServices::on_actor_released)
/// after every foreign release, and game-initiated releases clean the
/// arena directly. Behavior matches the donor: entity continuations,
/// monster contexts, perception state and source-slot cooldowns all
/// observe the release exactly once.

/// Shared body table surface used by Q2 (`SharedBodyTable`).
pub trait Q2BodyTable {
    /// Create a body record.
    fn create(&mut self, actor: &OwnedActor, initial: &BodyState);
    /// Read a body record.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body record.
    fn write(&mut self, actor: &OwnedActor, state: &BodyState);
    /// Attach a body to an anchor.
    fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment);
    /// Detach a body.
    fn detach(&mut self, actor: &OwnedActor);
    /// Read a body attachment.
    fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment>;
    /// Read the linked body snapshot.
    fn linked(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Link a body, optionally with a snapped collision origin.
    fn link(&mut self, actor: &OwnedActor, origin: Option<Vec3>);
    /// Unlink a body.
    fn unlink(&mut self, actor: &OwnedActor);
}

/// Actor callback table surface used by Q2 (`ActorCallbackTable`).
///
/// The donor binds per-actor closures here; the engine implementor routes
/// think/touch/use/pain/die dispatch through the Q2 game services
/// dispatch methods instead, and uses this table to track which actors
/// carry a Q2 continuation.
pub trait Q2CallbackTable {
    /// Record a Q2 continuation binding for an actor.
    fn bind(&mut self, actor: &OwnedActor);
    /// Drop a Q2 continuation binding.
    fn unbind(&mut self, actor: &ActorId);
    /// Whether an actor carries a Q2 continuation binding.
    fn is_bound(&self, actor: &ActorId) -> bool;
    /// Invoke the bound use continuation for an actor.
    ///
    /// The engine implementor forwards this to the Q2 services'
    /// `dispatch_use`; game code calls `dispatch_use` directly.
    fn forward_use(&mut self, actor: &OwnedActor, other: Option<&ActorId>, activator: Option<&ActorId>);
}

/// Gameplay combat authority surface used by Q2 (`GameplayAuthority`).
pub trait Q2CombatAuthority {
    /// Create a combat record.
    fn create(&mut self, actor: &OwnedActor, initial: &CombatState);
    /// Read a combat record.
    fn read(&self, actor: &ActorId) -> Option<CombatState>;
    /// Set health.
    fn set_health(&mut self, actor: &OwnedActor, health: f64);
    /// Set armor.
    fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState);
    /// Set regular armor points.
    fn set_regular_points(&mut self, actor: &OwnedActor, points: f64, initial: Option<&RegularArmorState>);
    /// Set regular armor.
    fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState);
    /// Set powered protection.
    fn set_powered_protection(&mut self, actor: &OwnedActor, powered: &PoweredProtectionState);
    /// Set combat traits.
    fn set_traits(&mut self, actor: &OwnedActor, changes: &CombatTraitChanges);
    /// Bind power armor cell storage.
    fn bind_power_armor_cells(&mut self, actor: &OwnedActor, cells: Box<dyn PowerArmorCells>);
    /// Apply a damage request.
    fn apply(&mut self, input: &DamageRequest) -> DamageOutcome;
}

/// Shared inventory table surface used by Q2 (`SharedInventoryTable`).
pub trait Q2InventoryTable {
    /// Create inventory entries for an actor.
    fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]);
    /// Read all entries for an actor.
    fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry>;
    /// Whether an actor has any inventory record.
    fn has(&self, actor: &ActorId) -> bool;
    /// Read an item count.
    fn count(&self, actor: &ActorId, item: &ItemId) -> f64;
    /// Consume an item count, reporting success.
    fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool;
    /// Give an item count, returning the count actually granted.
    fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64;
    /// Configure (admit) an inventory entry.
    fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry);
    /// Adjust a source counter, returning the new value.
    fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64;
}
