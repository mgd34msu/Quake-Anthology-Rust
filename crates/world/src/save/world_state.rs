//! Shared world-state capture and restore ported from `src/persistence/world-state.ts`.
//!
//! Bodies, attachments, combat, and inventories restore through a
//! [`SharedWorldHost`] so both the live simulation tables and synthetic
//! fixtures can drive the same coverage checks: exact prebound values
//! must equal the save, source-reconstructed actors keep provider-bound
//! state, and copied actors are created fresh. Deferred protection
//! ([`SharedRestoreCompletion`]) finishes after component source state is
//! attached, exactly like the donor's `deferProtection` flow.

use std::collections::{HashMap, HashSet};

use qa_core::identity::SavedActorId;
use qa_core::math::Bounds;

use super::ownership::ProviderCheckpoint;
use super::protection::{read_primary_protection, HiddenArmorEntry};
use super::records::{SavedBodyAttachment, UnifiedBody};
use super::source_items::read_source_items;
use super::value::save_error;
use crate::combat::{ArmorState, CombatState, PoweredProtection, RegularArmor};
use crate::inventory::InventoryEntry;
use crate::registry::{ActorSlotCheckpoint, SlotLifetime};
use crate::session::SavedBodyState;
use crate::WorldError;

/// How an actor's shared state is stored across the save boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    /// Values are copied into the save and recreated on restore.
    Copied,
    /// Values are prebound by the provider and must equal the save.
    Prebound,
    /// Values are reconstructed from source files by the provider.
    SourceReconstructed,
}

/// Protection channel for hidden-armor coverage checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionChannel {
    /// Regular armor channel.
    Regular,
    /// Powered protection channel.
    Powered,
}

/// Shared world-state view of a unified save.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SharedSaveView {
    /// Actor slots.
    pub actors: Vec<ActorSlotCheckpoint>,
    /// Body records.
    pub bodies: Vec<UnifiedBody>,
    /// Combat records.
    pub combat: Vec<(SavedActorId, CombatState)>,
    /// Inventory records.
    pub inventories: Vec<(SavedActorId, Vec<InventoryEntry>)>,
    /// Provider records.
    pub providers: Vec<ProviderCheckpoint>,
    /// Legacy (schema 2) armor layout flag.
    pub legacy_armor_layout: bool,
}

/// Host tables for shared-state restore.
pub trait SharedWorldHost {
    /// Whether an actor slot is live.
    fn is_active(&self, actor: SavedActorId) -> bool;
    /// Owner (`namespace:name`) of a live actor.
    fn owner(&self, actor: SavedActorId) -> Option<String>;
    /// Storage kind of a live actor.
    fn storage(&self, actor: SavedActorId) -> StorageKind;
    /// Live body state, if bound.
    fn body(&self, actor: SavedActorId) -> Option<SavedBodyState>;
    /// Create a copied body.
    fn create_body(&mut self, actor: SavedActorId, state: SavedBodyState) -> Result<(), WorldError>;
    /// Attach a body to its anchor.
    fn attach(&mut self, actor: SavedActorId, attachment: &SavedBodyAttachment) -> Result<(), WorldError>;
    /// Restore link state after provider collision metadata exists.
    fn restore_link(
        &mut self,
        actor: SavedActorId,
        link_count: u64,
        linked: Option<(SavedBodyState, Bounds)>,
    ) -> Result<(), WorldError>;
    /// Live combat state, if bound.
    fn combat(&self, actor: SavedActorId) -> Option<CombatState>;
    /// Create a copied combat state.
    fn create_combat(&mut self, actor: SavedActorId, state: CombatState);
    /// Overwrite powered protection (deferred completion).
    fn set_powered(&mut self, actor: SavedActorId, powered: PoweredProtection);
    /// Normalize legacy armor placeholders for a source-owned actor.
    fn normalize_legacy_armor(&self, actor: SavedActorId, armor: &ArmorState) -> ArmorState;
    /// Copied primary armor retained by the host, if any.
    fn copied_primary_armor(&self, actor: SavedActorId) -> Option<ArmorState>;
    /// Whether component protection owns a channel.
    fn protection_owned(&self, actor: SavedActorId, channel: ProtectionChannel) -> bool;
    /// Component protection inventory items for a channel.
    fn protection_items(&self, actor: SavedActorId, channel: ProtectionChannel) -> Vec<String>;
    /// Whether an inventory is bound.
    fn inventory_has(&self, actor: SavedActorId) -> bool;
    /// Live inventory entries.
    fn inventory_entries(&self, actor: SavedActorId) -> Vec<InventoryEntry>;
    /// Create a copied inventory.
    fn create_inventory(&mut self, actor: SavedActorId, entries: Vec<InventoryEntry>);
}

fn missing_actor(saved: SavedActorId) -> WorldError {
    save_error(
        "world",
        &format!("missing saved actor {}/{}", saved.slot, saved.generation),
    )
}

fn check_actor<H: SharedWorldHost>(host: &H, saved: SavedActorId) -> Result<SavedActorId, WorldError> {
    if host.is_active(saved) {
        Ok(saved)
    } else {
        Err(missing_actor(saved))
    }
}

/// Capture shared bodies from caller-supplied views.
#[must_use]
pub fn capture_shared_bodies(bodies: Vec<UnifiedBody>) -> Vec<UnifiedBody> {
    bodies
}

/// Restore shared body link state (runs after provider collision metadata exists).
pub fn restore_shared_body_links<H: SharedWorldHost>(bodies: &[UnifiedBody], host: &mut H) -> Result<(), WorldError> {
    for entry in bodies {
        let actor = check_actor(host, entry.actor).map_err(|_| {
            save_error(
                "world.body-links",
                &format!("missing saved actor {}/{}", entry.actor.slot, entry.actor.generation),
            )
        })?;
        if host.storage(actor) == StorageKind::SourceReconstructed {
            if host.body(actor).is_none() {
                return Err(save_error(
                    "world.body-links",
                    "reconstructed source body has not been bound",
                ));
            }
            continue;
        }
        if let Some((state, _)) = &entry.linked {
            if let Some(ground) = state.ground {
                check_actor(host, ground).map_err(|_| {
                    save_error(
                        "world.body-links",
                        &format!("missing saved actor {}/{}", ground.slot, ground.generation),
                    )
                })?;
            }
        }
        host.restore_link(actor, entry.link_count, entry.linked.clone())?;
    }
    Ok(())
}

fn counts_equal(left: f64, right: f64) -> bool {
    left == right || (left.is_nan() && right.is_nan())
}

/// Deferred shared-restoration completion.
#[derive(Debug)]
pub struct SharedRestoreCompletion<'h, H: SharedWorldHost> {
    host: &'h mut H,
    save: SharedSaveView,
    hidden: HashMap<(u32, u32), HiddenArmorEntry>,
    hidden_recorded: bool,
    state: CompletionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionState {
    Pending,
    Complete,
    Failed,
}

impl<H: SharedWorldHost> SharedRestoreCompletion<'_, H> {
    /// Finish after component source state and combat layers are attached.
    pub fn finish(&mut self) -> Result<(), WorldError> {
        if self.state != CompletionState::Pending {
            let state = match self.state {
                CompletionState::Complete => "complete",
                CompletionState::Failed => "failed",
                CompletionState::Pending => "pending",
            };
            return Err(save_error("world.combat", &format!("shared restoration is {state}")));
        }
        let result = self.finish_inner();
        self.state = if result.is_ok() {
            CompletionState::Complete
        } else {
            CompletionState::Failed
        };
        result
    }

    fn finish_inner(&mut self) -> Result<(), WorldError> {
        let mut copied: Vec<(SavedActorId, CombatState)> = Vec::new();
        for (saved, state) in &self.save.combat {
            let actor = check_actor(self.host, *saved)?;
            let storage = self.host.storage(actor);
            let expected = if self.save.legacy_armor_layout && storage != StorageKind::Copied {
                CombatState {
                    armor: self.host.normalize_legacy_armor(actor, &state.armor),
                    ..state.clone()
                }
            } else {
                state.clone()
            };
            let current = self
                .host
                .combat(actor)
                .ok_or_else(|| save_error("world.combat", "restored combat view has not been bound"))?;
            let external_regular = self.host.protection_owned(actor, ProtectionChannel::Regular);
            let external_power = self.host.protection_owned(actor, ProtectionChannel::Powered);
            let regular_items = self.host.protection_items(actor, ProtectionChannel::Regular);
            let powered_items = self.host.protection_items(actor, ProtectionChannel::Powered);
            let primary = self.hidden.get(&(saved.slot, saved.generation));
            let owns_copy = self.host.copied_primary_armor(actor).is_some();
            if primary.is_some() && !owns_copy {
                return Err(save_error(
                    "world.combat",
                    "source binding cannot restore copied primary armor",
                ));
            }
            if owns_copy && (self.hidden_recorded || external_regular) {
                let regular_covered = primary.map(|entry| entry.regular.is_some()).unwrap_or(false);
                let powered_covered = primary.map(|entry| entry.powered.is_some()).unwrap_or(false);
                if regular_covered != external_regular || powered_covered != external_power {
                    return Err(save_error(
                        "world.combat",
                        "hidden primary armor coverage differs from restored component ownership",
                    ));
                }
            }
            if storage == StorageKind::SourceReconstructed {
                for (channel, name, items) in [
                    (ProtectionChannel::Regular, "regular", &regular_items),
                    (ProtectionChannel::Powered, "powered", &powered_items),
                ] {
                    if self.host.protection_owned(actor, channel) {
                        let agrees = match channel {
                            ProtectionChannel::Regular => current.armor.regular == expected.armor.regular,
                            ProtectionChannel::Powered => current.armor.powered == expected.armor.powered,
                        };
                        if !agrees {
                            return Err(save_error(
                                "world.combat",
                                &format!("component {name} protection disagrees with saved shared state"),
                            ));
                        }
                    }
                    for item in items {
                        let saved_count = self
                            .save
                            .inventories
                            .iter()
                            .find(|(id, _)| *id == actor)
                            .and_then(|(_, rows)| rows.iter().find(|row| &row.item == item))
                            .map(|row| row.count);
                        let current_count = self
                            .host
                            .inventory_entries(actor)
                            .iter()
                            .find(|row| &row.item == item)
                            .map(|row| row.count);
                        match (saved_count, current_count) {
                            (Some(left), Some(right)) if counts_equal(left, right) => {}
                            _ => {
                                return Err(save_error(
                                    "world.inventories",
                                    "component protection inventory disagrees with saved shared state",
                                ));
                            }
                        }
                    }
                }
            } else if storage == StorageKind::Copied && !external_power {
                let merged = CombatState {
                    armor: ArmorState {
                        powered: expected.armor.powered.clone(),
                        ..current.armor.clone()
                    },
                    ..current.clone()
                };
                if merged != expected {
                    return Err(save_error(
                        "world.combat",
                        "copied combat disagrees with saved shared state",
                    ));
                }
                copied.push((actor, expected));
            } else if current != expected {
                return Err(save_error(
                    "world.combat",
                    "source combat disagrees with saved shared state",
                ));
            }
        }
        for (saved, rows) in &self.save.inventories {
            let actor = check_actor(self.host, *saved)?;
            if self.host.storage(actor) != StorageKind::SourceReconstructed
                && (!self.host.inventory_has(actor) || self.host.inventory_entries(actor) != *rows)
            {
                return Err(save_error(
                    "world.inventories",
                    "restored inventory disagrees with saved shared state",
                ));
            }
        }
        for (actor, state) in copied {
            self.host.set_powered(actor, state.armor.powered.clone());
            if self.host.combat(actor).as_ref() != Some(&state) {
                return Err(save_error(
                    "world.combat",
                    "copied powered protection disagrees with saved shared state",
                ));
            }
        }
        Ok(())
    }

    /// Assert the deferred restore completed.
    pub fn assert_complete(&self) -> Result<(), WorldError> {
        if self.state != CompletionState::Complete {
            let state = match self.state {
                CompletionState::Complete => "complete",
                CompletionState::Failed => "failed",
                CompletionState::Pending => "pending",
            };
            return Err(save_error("world.combat", &format!("shared restoration is {state}")));
        }
        Ok(())
    }
}

/// Restore shared world state.
///
/// With `defer_protection`, hidden primary armor is staged and the
/// returned completion must be finished after component source state is
/// attached. `hydrate` runs source-graph hydration between combat and
/// inventory restoration when provided.
pub fn restore_shared_world_state<'h, H: SharedWorldHost>(
    save: &SharedSaveView,
    host: &'h mut H,
    hydrate: Option<&mut dyn FnMut(&mut H)>,
    defer_protection: bool,
) -> Result<Option<SharedRestoreCompletion<'h, H>>, WorldError> {
    let hidden = read_primary_protection(&save.providers, &|saved| {
        host.owner(saved)
            .map(|owner| super::protection::ProtectionOwner { owner })
    })?;
    let source_items = read_source_items(
        &save.providers,
        &save
            .inventories
            .iter()
            .map(|(actor, rows)| (*actor, rows.clone()))
            .collect::<Vec<_>>(),
    )?;
    let primary_rows: HashMap<(u32, u32), &[InventoryEntry]> = source_items
        .iter()
        .map(|record| ((record.actor.slot, record.actor.generation), record.primary.as_slice()))
        .collect();
    let body_actors: HashSet<(u32, u32)> = save
        .bodies
        .iter()
        .map(|entry| (entry.actor.slot, entry.actor.generation))
        .collect();
    let combat_actors: HashSet<(u32, u32)> = save
        .combat
        .iter()
        .map(|(actor, _)| (actor.slot, actor.generation))
        .collect();
    for key in hidden.entries.keys() {
        let saved = SavedActorId {
            slot: key.0,
            generation: key.1,
        };
        if !combat_actors.contains(key) || !host.is_active(saved) || host.storage(saved) != StorageKind::Copied {
            return Err(save_error(
                "world.combat",
                "hidden armor checkpoint requires a copied combat owner",
            ));
        }
    }
    let inventories: HashMap<(u32, u32), &[InventoryEntry]> = save
        .inventories
        .iter()
        .map(|(actor, rows)| ((actor.slot, actor.generation), rows.as_slice()))
        .collect();
    for entry in &save.actors {
        if !matches!(entry.lifetime, SlotLifetime::Active { .. }) {
            continue;
        }
        let saved = SavedActorId {
            slot: entry.slot,
            generation: entry.generation,
        };
        let restored = check_actor(host, saved)?;
        if host.storage(restored) == StorageKind::Copied {
            continue;
        }
        let key = (saved.slot, saved.generation);
        if (host.body(restored).is_some()) != body_actors.contains(&key)
            || (host.combat(restored).is_some()) != combat_actors.contains(&key)
            || host.inventory_has(restored) != inventories.contains_key(&key)
        {
            return Err(save_error(
                "world",
                "source bindings disagree with saved shared state coverage",
            ));
        }
    }
    for entry in &save.bodies {
        let restored = check_actor(host, entry.actor)?;
        let storage = host.storage(restored);
        if storage != StorageKind::Copied {
            let body = host
                .body(restored)
                .ok_or_else(|| save_error("world.bodies", "source body view has not been bound"))?;
            if storage == StorageKind::SourceReconstructed {
                continue;
            }
            if let Some(ground) = entry.body.ground {
                check_actor(host, ground)?;
            }
            if body != entry.body {
                return Err(save_error(
                    "world.bodies",
                    "source body disagrees with saved shared state",
                ));
            }
        } else {
            if let Some(ground) = entry.body.ground {
                check_actor(host, ground)?;
            }
            host.create_body(restored, entry.body.clone())?;
        }
    }
    for entry in save.bodies.iter().filter(|entry| entry.attachment.is_some()) {
        let attachment = entry.attachment.as_ref().expect("filtered attachment");
        check_actor(host, attachment.anchor)?;
        host.attach(check_actor(host, entry.actor)?, attachment)?;
    }
    for (saved, state) in &save.combat {
        let restored = check_actor(host, *saved)?;
        let storage = host.storage(restored);
        let expected = if !defer_protection && save.legacy_armor_layout && storage != StorageKind::Copied {
            CombatState {
                armor: host.normalize_legacy_armor(restored, &state.armor),
                ..state.clone()
            }
        } else {
            state.clone()
        };
        if storage != StorageKind::Copied {
            let current = host
                .combat(restored)
                .ok_or_else(|| save_error("world.combat", "source combat view has not been bound"))?;
            if !defer_protection && storage == StorageKind::Prebound && current != expected {
                return Err(save_error(
                    "world.combat",
                    "source combat disagrees with saved shared state",
                ));
            }
        } else {
            let primary = hidden.entries.get(&(saved.slot, saved.generation));
            if !defer_protection && primary.is_some() {
                return Err(save_error(
                    "world.combat",
                    "hidden primary armor requires component restoration",
                ));
            }
            host.create_combat(
                restored,
                if defer_protection {
                    CombatState {
                        armor: ArmorState {
                            regular: primary.and_then(|entry| entry.regular.clone()).unwrap_or_else(|| {
                                if matches!(expected.armor.regular, RegularArmor::Source { .. }) {
                                    RegularArmor::None
                                } else {
                                    expected.armor.regular.clone()
                                }
                            }),
                            powered: primary
                                .and_then(|entry| entry.powered.clone())
                                .unwrap_or(PoweredProtection::None),
                        },
                        ..expected.clone()
                    }
                } else {
                    expected
                },
            );
        }
    }
    if let Some(hydrate) = hydrate {
        hydrate(host);
    }
    for (saved, rows) in &save.inventories {
        let restored = check_actor(host, *saved)?;
        let storage = host.storage(restored);
        let primary = primary_rows
            .get(&(saved.slot, saved.generation))
            .map_or(rows.as_slice(), |rows| rows);
        if storage != StorageKind::Copied {
            if !host.inventory_has(restored) {
                return Err(save_error(
                    "world.inventories",
                    "source inventory view has not been bound",
                ));
            }
            if storage == StorageKind::Prebound && host.inventory_entries(restored).as_slice() != primary {
                return Err(save_error(
                    "world.inventories",
                    "source inventory disagrees with saved shared state",
                ));
            }
        } else {
            host.create_inventory(restored, primary.to_vec());
        }
    }
    if !defer_protection {
        return Ok(None);
    }
    Ok(Some(SharedRestoreCompletion {
        host,
        save: save.clone(),
        hidden: hidden.entries,
        hidden_recorded: hidden.recorded,
        state: CompletionState::Pending,
    }))
}

/// Owned staged shared-restore completion (C10: defer/finish across the load boundary).
///
/// [`restore_shared_world_state`] borrows its host, but the simulation stores
/// the pending completion in its state while the load flow attaches component
/// state with a fresh host. Staging keeps the owned payload; finishing
/// re-attaches a host.
#[derive(Debug, Clone)]
pub struct StagedSharedRestore {
    save: SharedSaveView,
    hidden: HashMap<(u32, u32), HiddenArmorEntry>,
    hidden_recorded: bool,
    state: CompletionState,
}

impl<H: SharedWorldHost> SharedRestoreCompletion<'_, H> {
    /// Detach the owned staged payload, dropping the host borrow.
    pub fn into_staged(self) -> StagedSharedRestore {
        StagedSharedRestore {
            save: self.save,
            hidden: self.hidden,
            hidden_recorded: self.hidden_recorded,
            state: self.state,
        }
    }
}

/// Finish a staged restore with a fresh host (C10).
pub fn finish_staged_restore<H: SharedWorldHost>(
    host: &mut H,
    staged: &mut StagedSharedRestore,
) -> Result<(), WorldError> {
    let mut completion = SharedRestoreCompletion {
        host,
        save: staged.save.clone(),
        hidden: staged.hidden.clone(),
        hidden_recorded: staged.hidden_recorded,
        state: staged.state,
    };
    let result = completion.finish();
    staged.state = completion.state;
    result
}

/// Assert a staged restore completed (C10).
pub fn assert_staged_complete(staged: &StagedSharedRestore) -> Result<(), WorldError> {
    if staged.state != CompletionState::Complete {
        let state = match staged.state {
            CompletionState::Complete => "complete",
            CompletionState::Failed => "failed",
            CompletionState::Pending => "pending",
        };
        return Err(save_error("world.combat", &format!("shared restoration is {state}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::{ArmorState, PoweredProtection};
    use crate::registry::ActorSlotCheckpoint;
    use qa_core::identity::ProviderId;
    use qa_core::math::{Bounds, Vec3};

    type LinkState = (u64, Option<(SavedBodyState, Bounds)>);

    #[derive(Default)]
    struct FixtureHost {
        active: HashSet<(u32, u32)>,
        owners: HashMap<(u32, u32), String>,
        storage: HashMap<(u32, u32), StorageKind>,
        bodies: HashMap<(u32, u32), SavedBodyState>,
        links: HashMap<(u32, u32), LinkState>,
        combat: HashMap<(u32, u32), CombatState>,
        inventories: HashMap<(u32, u32), Vec<InventoryEntry>>,
    }

    fn key(actor: SavedActorId) -> (u32, u32) {
        (actor.slot, actor.generation)
    }

    impl SharedWorldHost for FixtureHost {
        fn is_active(&self, actor: SavedActorId) -> bool {
            self.active.contains(&key(actor))
        }
        fn owner(&self, actor: SavedActorId) -> Option<String> {
            self.owners.get(&key(actor)).cloned()
        }
        fn storage(&self, actor: SavedActorId) -> StorageKind {
            self.storage.get(&key(actor)).copied().unwrap_or(StorageKind::Copied)
        }
        fn body(&self, actor: SavedActorId) -> Option<SavedBodyState> {
            self.bodies.get(&key(actor)).cloned()
        }
        fn create_body(&mut self, actor: SavedActorId, state: SavedBodyState) -> Result<(), WorldError> {
            self.bodies.insert(key(actor), state);
            Ok(())
        }
        fn attach(&mut self, _actor: SavedActorId, _attachment: &SavedBodyAttachment) -> Result<(), WorldError> {
            Ok(())
        }
        fn restore_link(
            &mut self,
            actor: SavedActorId,
            link_count: u64,
            linked: Option<(SavedBodyState, Bounds)>,
        ) -> Result<(), WorldError> {
            self.links.insert(key(actor), (link_count, linked));
            Ok(())
        }
        fn combat(&self, actor: SavedActorId) -> Option<CombatState> {
            self.combat.get(&key(actor)).cloned()
        }
        fn create_combat(&mut self, actor: SavedActorId, state: CombatState) {
            self.combat.insert(key(actor), state);
        }
        fn set_powered(&mut self, actor: SavedActorId, powered: PoweredProtection) {
            if let Some(state) = self.combat.get_mut(&key(actor)) {
                state.armor.powered = powered;
            }
        }
        fn normalize_legacy_armor(&self, _actor: SavedActorId, armor: &ArmorState) -> ArmorState {
            armor.clone()
        }
        fn copied_primary_armor(&self, _actor: SavedActorId) -> Option<ArmorState> {
            None
        }
        fn protection_owned(&self, _actor: SavedActorId, _channel: ProtectionChannel) -> bool {
            false
        }
        fn protection_items(&self, _actor: SavedActorId, _channel: ProtectionChannel) -> Vec<String> {
            Vec::new()
        }
        fn inventory_has(&self, actor: SavedActorId) -> bool {
            self.inventories.contains_key(&key(actor))
        }
        fn inventory_entries(&self, actor: SavedActorId) -> Vec<InventoryEntry> {
            self.inventories.get(&key(actor)).cloned().unwrap_or_default()
        }
        fn create_inventory(&mut self, actor: SavedActorId, entries: Vec<InventoryEntry>) {
            self.inventories.insert(key(actor), entries);
        }
    }

    fn sample_body() -> SavedBodyState {
        SavedBodyState {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            bounds: Bounds {
                min: Vec3 {
                    x: -1.0,
                    y: -1.0,
                    z: -1.0,
                },
                max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            },
            ground: None,
        }
    }

    fn sample_save() -> SharedSaveView {
        let actor = SavedActorId { slot: 0, generation: 0 };
        SharedSaveView {
            actors: vec![ActorSlotCheckpoint {
                slot: 0,
                generation: 0,
                lifetime: crate::registry::SlotLifetime::Active {
                    owner: ProviderId::new("q3", "game"),
                    definition: "q3:soldier".to_string(),
                },
            }],
            bodies: vec![UnifiedBody {
                actor,
                body: sample_body(),
                attachment: None,
                link_count: 2,
                linked: None,
            }],
            combat: vec![(actor, CombatState::default())],
            inventories: vec![(
                actor,
                vec![InventoryEntry {
                    item: "q3:shells".to_string(),
                    count: 5.0,
                    capacity: 100.0,
                    count_policy: None,
                }],
            )],
            providers: Vec::new(),
            legacy_armor_layout: false,
        }
    }

    fn active_host() -> FixtureHost {
        let mut host = FixtureHost::default();
        host.active.insert((0, 0));
        host.owners.insert((0, 0), "q3:game".to_string());
        host
    }

    #[test]
    fn copied_world_round_trip() {
        let save = sample_save();
        let mut host = active_host();
        let completion = restore_shared_world_state(&save, &mut host, None, false).unwrap();
        assert!(completion.is_none());
        assert_eq!(host.body(SavedActorId { slot: 0, generation: 0 }), Some(sample_body()));
        assert_eq!(
            host.combat(SavedActorId { slot: 0, generation: 0 }),
            Some(CombatState::default())
        );
        assert_eq!(host.inventory_entries(SavedActorId { slot: 0, generation: 0 }).len(), 1);
        restore_shared_body_links(&save.bodies, &mut host).unwrap();
        assert_eq!(host.links[&(0, 0)].0, 2);
    }

    #[test]
    fn prebound_mismatches_fail() {
        let save = sample_save();
        let mut host = active_host();
        host.storage.insert((0, 0), StorageKind::Prebound);
        host.bodies.insert((0, 0), sample_body());
        host.combat.insert((0, 0), CombatState::default());
        host.inventories.insert((0, 0), vec![]);
        assert!(restore_shared_world_state(&save, &mut host, None, false).is_err());

        let mut host = active_host();
        host.storage.insert((0, 0), StorageKind::SourceReconstructed);
        assert!(restore_shared_world_state(&sample_save(), &mut host, None, false).is_err());
    }

    #[test]
    fn deferred_completion_requires_finish() {
        let save = sample_save();
        let mut host = active_host();
        let mut completion = restore_shared_world_state(&save, &mut host, None, true)
            .unwrap()
            .unwrap();
        assert!(completion.assert_complete().is_err());
        completion.finish().unwrap();
        completion.assert_complete().unwrap();
        assert!(completion.finish().is_err());
    }
}
