//! Aggregate primary profiles: the complete original-source player interface.
//!
//! Provenance: `src/compat/qvm/primary-profile.ts`.
//!
//! An external primary is admitted whole: input, weapons, inventory, pickups,
//! and combat must agree on their player records and movement entries, and the
//! pickup item table must match the primary catalog. The sub-profile shapes are
//! the local mirrors owned by the sibling reader modules of this assignment
//! (`primary-player-profile.rs`, `primary-inventory-profile.rs`,
//! `primary-pickup-profile.rs`); the sibling `game-*` modules own the runtime
//! shapes these readers feed.
//!
//! Local mirrors defined here: [`ResolvedResourceReference`] (carried
//! declaration identity; the content worker owns the full resource shape with
//! provenance and resolution).

use std::collections::HashSet;

use super::mod_provider::{ProfileReader, QvmArtifact};
use super::primary_inventory_profile::{PrimaryRecordStrides, QvmInventoryProfile, read_qvm_primary_inventory_profile};
use super::primary_pickup_profile::{PickupOperation, QvmItemLayout, QvmPickupProfile, WeaponGrantLocation, read_qvm_primary_pickup_profile};
use super::primary_player_profile::{
    DropAmmo, QvmInputDefinition, QvmPrimaryCombatProfile, QvmPrimaryWeaponProfile, QvmWeaponCatalogRow, read_qvm_primary_combat,
    read_qvm_primary_input, read_qvm_primary_weapons,
};
use crate::error::GuestError;

/// Carried declaration identity (mirror of `ResolvedResourceReference`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedResourceReference {
    /// Resource id.
    pub id: String,
    /// Requested path.
    pub requested_path: String,
    /// Content digest.
    pub digest: String,
    /// Byte length.
    pub byte_length: usize,
}

/// Aggregate primary profile.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPrimaryProfile {
    /// Declaration, if externally supplied.
    pub declaration: Option<ResolvedResourceReference>,
    /// Input definition, if present.
    pub input: Option<QvmInputDefinition>,
    /// Weapon profile, if present.
    pub weapons: Option<QvmPrimaryWeaponProfile>,
    /// Inventory profile, if present.
    pub inventory: Option<QvmInventoryProfile>,
    /// Pickup profile, if present.
    pub pickups: Option<QvmPickupProfile>,
    /// Combat profile, if present.
    pub combat: Option<QvmPrimaryCombatProfile>,
}

/// Stock-game constructors backing [`builtin_qvm_primary_profile`].
pub trait BuiltinPrimarySource {
    /// Stock input definition.
    fn builtin_input(&self, artifact: &QvmArtifact) -> QvmInputDefinition;
    /// Stock weapon profile.
    fn builtin_weapons(&self, artifact: &QvmArtifact, catalog: &[QvmWeaponCatalogRow]) -> QvmPrimaryWeaponProfile;
    /// Stock inventory profile.
    fn builtin_inventory(&self, artifact: &QvmArtifact) -> QvmInventoryProfile;
    /// Stock pickup profile.
    fn builtin_pickups(&self, artifact: &QvmArtifact) -> QvmPickupProfile;
    /// Stock combat profile.
    fn builtin_combat(&self, artifact: &QvmArtifact) -> QvmPrimaryCombatProfile;
}

/// Build the builtin stock primary profile.
pub fn builtin_qvm_primary_profile<S: BuiltinPrimarySource + ?Sized>(
    source: &S,
    artifact: &QvmArtifact,
    catalog: &[QvmWeaponCatalogRow],
) -> QvmPrimaryProfile {
    QvmPrimaryProfile {
        declaration: None,
        input: Some(source.builtin_input(artifact)),
        weapons: Some(source.builtin_weapons(artifact, catalog)),
        inventory: Some(source.builtin_inventory(artifact)),
        pickups: Some(source.builtin_pickups(artifact)),
        combat: Some(source.builtin_combat(artifact)),
    }
}

/// Check that the five interfaces agree on their player records, movement
/// entries, item table, and inventory projection.
pub fn check_qvm_primary_consistency(
    reader: &ProfileReader<'_>,
    input: &QvmInputDefinition,
    weapons: &QvmPrimaryWeaponProfile,
    inventory: &QvmInventoryProfile,
    pickups: &QvmPickupProfile,
    combat: &QvmPrimaryCombatProfile,
    items: &QvmItemLayout,
) -> Result<(), GuestError> {
    if input.entity_stride != weapons.entity_stride
        || input.client_stride != weapons.client_stride
        || input.client_pointer != weapons.client_pointer
        || input.entity_stride != pickups.entity_stride
        || input.client_stride != pickups.client_stride
        || input.client_pointer != pickups.fields.client
        || input.entity_stride != combat.entity_stride
        || input.client_stride != combat.client_stride
        || input.client_pointer != combat.fields.client
        || input.entries.move_ != weapons.equipment_movement.move_
        || input.entries.slice != weapons.equipment_movement.slice
    {
        return reader.fail("primary interfaces disagree about their original player records or movement entries");
    }
    if items != &pickups.items {
        return reader.field("items")?.fail("primary catalog and pickup interfaces name different item tables");
    }
    match inventory {
        QvmInventoryProfile::Private { storage, .. } => {
            let stored: HashSet<&str> = storage.iter().flat_map(|entry| entry.stored_items()).collect();
            if weapons.stage.selection.values.iter().any(|value| !stored.contains(value.item.as_str())) {
                return reader.field("inventory")?.fail("original weapon selection lacks private inventory storage");
            }
            let drops_private = matches!(weapons.drop.ammo, DropAmmo::Inventory);
            let grants_private = pickups.grants.iter().all(|grant| match &grant.operation {
                PickupOperation::Region { weapon: Some(weapon), .. } => matches!(weapon.location, WeaponGrantLocation::Inventory),
                _ => true,
            });
            if !drops_private || !grants_private {
                return reader
                    .field("inventory")?
                    .fail("private inventory requires original pickup/drop projection through its declared storage");
            }
        }
        QvmInventoryProfile::Public { .. } => {
            if weapons.stage.selection.values.iter().any(|value| value.value > 15) {
                return reader.field("inventory")?.fail("original weapon selection exceeds public inventory storage");
            }
        }
    }
    Ok(())
}

/// Read an external primary profile as a complete interface.
pub fn read_qvm_primary_profile(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
    catalog: Option<&[QvmWeaponCatalogRow]>,
    items: &QvmItemLayout,
    declaration: ResolvedResourceReference,
) -> Result<QvmPrimaryProfile, GuestError> {
    let input = read_qvm_primary_input(&reader.field("input")?, artifact)?;
    let weapons = read_qvm_primary_weapons(&reader.field("weapons")?, artifact, catalog)?;
    let records = PrimaryRecordStrides { client_stride: weapons.client_stride, entity_stride: weapons.entity_stride };
    let inventory = read_qvm_primary_inventory_profile(&reader.field("inventory")?, artifact, Some(records))?;
    let pickups = read_qvm_primary_pickup_profile(&reader.field("pickups")?, artifact)?;
    let combat = read_qvm_primary_combat(&reader.field("combat")?, artifact)?;
    check_qvm_primary_consistency(reader, &input, &weapons, &inventory, &pickups, &combat, items)?;
    Ok(QvmPrimaryProfile { declaration: Some(declaration), input: Some(input), weapons: Some(weapons), inventory: Some(inventory), pickups: Some(pickups), combat: Some(combat) })
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{InputPointerKind, ModuleId, ProfileValue, QvmAbi, QvmImage, QvmModInputPointer, QvmRegionEvaluation};
    use super::super::mod_weapon_stage::{
        DispatcherHead, QvmItemCapacity, QvmItemField, QvmItemStorage, QvmWeaponActor, QvmWeaponDispatcherDefinition, SelectionValue, StageRequest,
        StageSelection,
    };
    use super::super::primary_inventory_profile::InventoryCapacity;
    use super::super::primary_pickup_profile::{FunctionCalls, GateProfile, ItemTableFields, PickupFields, TableAddress, TableCount};
    use super::super::primary_player_profile::{
        ArmorDefinition, CombatCallbacks, CombatFields, CombatReactions, CombatState, CombatStateFlags, CombatTeamState, DamageFactor, DelayPlayer,
        DropProfile, GiveProfile, InputEntries, NamedGrant, PowerupOffsets, QvmCombatCall, QvmCombatMass, QvmDamageFlags, QvmEquipmentMovementProfile,
        QvmReactionCall, RegionRef, TeleportProfile, TorsoAnimation, WaterLevel, WeaponAvailability,
    };
    use super::*;

    const ENTITY: usize = 1024;
    const CLIENT: usize = 2048;
    const POINTER: usize = 100;

    fn module() -> ModuleId {
        ModuleId { id: "test:game".to_string(), artifact_path: "vm/qagame.qvm".to_string(), digest: "sha256:game".to_string(), revision: "1".to_string() }
    }

    fn region() -> QvmRegionEvaluation {
        QvmRegionEvaluation { entry: 1, join: 2, inputs: Vec::new(), result: None }
    }

    fn input() -> QvmInputDefinition {
        QvmInputDefinition {
            module: module(),
            entity_stride: ENTITY,
            client_stride: CLIENT,
            client_pointer: POINTER,
            intermission: Vec::new(),
            movement_modes: None,
            entries: InputEntries { client_think: 1, run_client: 2, client_spawn: 3, move_: 10, slice: 20 },
        }
    }

    fn weapons() -> QvmPrimaryWeaponProfile {
        QvmPrimaryWeaponProfile {
            module: module(),
            match_: None,
            abi_profile: QvmAbi::Modern,
            equipment_movement: QvmEquipmentMovementProfile {
                move_: 10,
                slice: 20,
                duck: 30,
                movement_global: 40,
                locomotion: RegionRef { entry: 1, join: 2 },
                mins: 50,
                maxs: 60,
                body_trace: None,
            },
            entity_stride: ENTITY,
            client_stride: CLIENT,
            client_pointer: POINTER,
            stage: QvmWeaponDispatcherDefinition {
                dispatcher: DispatcherHead {
                    entry: 1,
                    actor: QvmWeaponActor {
                        record: "entity".to_string(),
                        pointer: QvmModInputPointer { kind: InputPointerKind::Argument { index: 0 }, indirections: Vec::new(), offset: 0 },
                    },
                },
                predicates: Vec::new(),
                settled: Vec::new(),
                selection: StageSelection {
                    field: QvmItemField { record: "client".to_string(), offset: 8 },
                    values: vec![SelectionValue { value: 3, item: "weapon_rocket".to_string() }],
                },
                request: StageRequest { entry: 1, argument: 0, accepted: Vec::new() },
            },
            damage_factor: DamageFactor { entry: 1, result: 2, stop: RegionRef { entry: 1, join: 2 } },
            equipment_contexts: Vec::new(),
            delay: region(),
            delay_player: DelayPlayer { movement_global: 1, player_offset: 2 },
            teleport: TeleportProfile { entry: 1, region: region(), objectives: region(), spawn: 2, view: 3 },
            max_health: 4,
            persistent_max_health: 8,
            availability: WeaponAvailability {
                movement_type: 1,
                excluded: Vec::new(),
                health: 2,
                team: 3,
                spectator_team: 0,
                flags: 4,
                respawn_flag: 1,
            },
            powerups: PowerupOffsets { quad: 1, haste: 2, flight: 3 },
            torso_animation: TorsoAnimation { entry: 1, attack: 2, melee: 3 },
            water_level: WaterLevel { entity_offset: 1, movement_offset: 2 },
            drop: DropProfile { entry: 1, argument: 0, weapon: 2, ammo: DropAmmo::Inventory, region: RegionRef { entry: 1, join: 2 } },
            give: GiveProfile { entry: 1, argument: 0, weapons: 2, ammo: 3, named: NamedGrant { entry: 1, join: 2, name: 3, item: 4 } },
        }
    }

    fn inventory() -> QvmInventoryProfile {
        QvmInventoryProfile::Private {
            module: module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: ENTITY,
            client_stride: CLIENT,
            image: QvmImage { instructions: Vec::new(), data_length: 0, literal_length: 0, bss_length: 0, initialized_length: 0, allocated_data_length: 0 },
            storage: vec![QvmItemStorage::counter(
                QvmItemField { record: "client".to_string(), offset: 8 },
                "weapon_rocket".to_string(),
                QvmItemCapacity::Constant(10),
            )],
        }
    }

    fn layout() -> QvmItemLayout {
        QvmItemLayout {
            address: TableAddress::Direct(100),
            count: TableCount::Direct(8),
            live: false,
            stride: 64,
            fields: ItemTableFields { class_name: 0, pickup_name: 4, type_: 8, tag: 12 },
            weapon_type: 2,
            ammo_type: 4,
        }
    }

    fn pickups() -> QvmPickupProfile {
        QvmPickupProfile {
            module: module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: ENTITY,
            client_stride: CLIENT,
            fields: PickupFields { inuse: 0, client: POINTER, health: 4, item: 8, count: 12, flags: 16 },
            dropped_flag: 1,
            items: layout(),
            touch: 1,
            gate: GateProfile { entry: 1, calls: Vec::new(), item_argument: 0, player_argument: 1 },
            targets: FunctionCalls { entry: 1, calls: Vec::new() },
            free: 2,
            objective_types: Vec::new(),
            grants: Vec::new(),
        }
    }

    fn combat() -> QvmPrimaryCombatProfile {
        let call = QvmCombatCall::damage(0, 1, 2, 3, 4, 5, 6, 7, Vec::new());
        QvmPrimaryCombatProfile {
            damage_call: call.clone(),
            module: module(),
            abi_profile: QvmAbi::Modern,
            entity_stride: ENTITY,
            client_stride: CLIENT,
            fields: CombatFields { inuse: 0, health: 4, takedamage: 8, parent: 12, client: POINTER },
            callbacks: CombatCallbacks { allocate: 1, free: 2, damage: 3 },
            armor: ArmorDefinition { check_armor: 1, call, points_stat: 2, protection: 0.5, tiers: None },
            reactions: CombatReactions {
                flags: 1,
                pain: 2,
                die: 3,
                pain_call: QvmReactionCall { arguments: 2, target: 0, amount: 1 },
                die_call: QvmReactionCall { arguments: 2, target: 0, amount: 1 },
            },
            grapple_damage_method: 0,
            state: CombatState {
                health_stat: 1,
                team: CombatTeamState { persistent_stat: 2, values: Vec::new() },
                flags: CombatStateFlags { notarget: 1, invulnerable: 2, no_knockback: 4 },
                mass: QvmCombatMass::Constant(100.0),
            },
            damage_flags: QvmDamageFlags { radius: 1, no_armor: 2, no_knockback: 4, no_protection: 8, no_team_protection: 16 },
        }
    }

    fn check(parts: (&QvmInputDefinition, &QvmPrimaryWeaponProfile, &QvmInventoryProfile, &QvmPickupProfile, &QvmPrimaryCombatProfile, &QvmItemLayout)) -> Result<(), GuestError> {
        let root = ProfileValue::record(Vec::new());
        let reader = ProfileReader::new(&root);
        check_qvm_primary_consistency(reader, parts.0, parts.1, parts.2, parts.3, parts.4, parts.5)
    }

    #[test]
    fn consistent_parts_pass() {
        let input = input();
        let weapons = weapons();
        let inventory = inventory();
        let pickups = pickups();
        let combat = combat();
        let items = layout();
        check((&input, &weapons, &inventory, &pickups, &combat, &items)).expect("consistent parts");
    }

    #[test]
    fn stride_and_movement_disagreements_fail() {
        let input = input();
        let weapons = weapons();
        let inventory = inventory();
        let pickups = pickups();
        let combat = combat();
        let items = layout();
        let mut drifted = combat.clone();
        drifted.client_stride = CLIENT + 4;
        let error = check((&input, &weapons, &inventory, &pickups, &drifted, &items)).expect_err("stride drift");
        assert!(error.to_string().contains("original player records"), "unexpected: {error}");
        let mut moved = weapons.clone();
        moved.equipment_movement.slice = 21;
        let error = check((&input, &moved, &inventory, &pickups, &combat, &items)).expect_err("movement drift");
        assert!(error.to_string().contains("movement entries"), "unexpected: {error}");
    }

    #[test]
    fn catalog_mismatch_fails() {
        let input = input();
        let weapons = weapons();
        let inventory = inventory();
        let pickups = pickups();
        let combat = combat();
        let mut items = layout();
        items.stride = 72;
        let error = check((&input, &weapons, &inventory, &pickups, &combat, &items)).expect_err("catalog drift");
        assert!(error.to_string().contains("different item tables"), "unexpected: {error}");
    }

    #[test]
    fn private_inventory_requires_stored_selection_and_projection() {
        let input = input();
        let weapons = weapons();
        let inventory = inventory();
        let pickups = pickups();
        let combat = combat();
        let items = layout();
        let mut unstored = weapons.clone();
        unstored.stage.selection.values.push(SelectionValue { value: 5, item: "weapon_railgun".to_string() });
        let error = check((&input, &unstored, &inventory, &pickups, &combat, &items)).expect_err("unstored selection");
        assert!(error.to_string().contains("lacks private inventory storage"), "unexpected: {error}");
        let mut offset_drop = weapons.clone();
        offset_drop.drop.ammo = DropAmmo::Offset(12);
        let error = check((&input, &offset_drop, &inventory, &pickups, &combat, &items)).expect_err("offset drop");
        assert!(error.to_string().contains("projection through its declared st
...[truncated 2732 chars]