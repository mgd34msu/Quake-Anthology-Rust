//! Q2 items (`src/content/q2/foundation/items.ts`).
//!
//! Pickup and inventory behaviors adapted from Quake II game/g_items.c and p_weapon.c.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{add3, scale3, vec3};
use qa_core::time::SourceTime;

use super::callbacks::{Q2CallbackDefinitions, free_q2_entity};
use super::checkpoint::restore_q2_actor;
use super::fields::movedir;
use super::host::{
    Q2Edition, Q2EffectEvent, Q2GameServices, Q2ItemNameFn, Q2Mode, Q2MotionKind,
    Q2OriginalPickupContinuation, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2SpawnFn, Q2TraceRequest, SpawnModule,
};
use super::monsters::types::SharedPowerCells;
use super::start_items::parse_q2_start_items;
use super::weapons::definitions::base_weapons;
use super::weapons::types::Q2BaseWeaponName;
use crate::contract::{
    AmmoWeaponSelection, AmmoWeaponTiming, InventoryCountPolicy, InventoryEntry, ItemId,
    OriginalPickupOffer, PickupAdmission, PickupAmmoGrant, PickupAmmoWeaponGrant,
    PickupAvailability, PickupCount, PickupMapKind, PickupResource, PickupSelection,
    PickupSupplyObservation, PickupSupplyOffer, PickupSupplyPreview, PickupWeaponGrant,
    PoweredProtectionState, ProtectionChannel, RegularArmorState, SourceCounterArithmetic,
};
use crate::q2::support::contracts::{CombatTraitChanges, TouchContact};
use crate::q2::support::misc::{
    PickupAcceptance, PickupGrantPlan, PickupWeaponLink, preview_pickup_grants,
};

/// Custom item pickup (`Q2ItemDefinition` custom `pickup`).
pub type Q2CustomPickup = fn(ActorId, &mut Q2GameServices, OwnedActor) -> bool;

/// Custom item use (`Q2ItemDefinition` custom `use`).
pub type Q2CustomUse = fn(OwnedActor, &mut Q2GameServices) -> bool;

/// Item kind (`Q2ItemDefinition["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ItemKind {
    /// Ammo.
    Ammo,
    /// Weapon.
    Weapon,
    /// Health.
    Health,
    /// Armor.
    Armor,
    /// Armor shard.
    Shard,
    /// Powerup.
    Power,
    /// Power armor.
    PowerArmor,
    /// Maximum health.
    MaximumHealth,
    /// Key.
    Key,
    /// Ammo pack.
    AmmoPack,
    /// Custom item.
    Custom,
}

/// Console give policy (`consoleGive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ConsoleGive {
    /// Normal pickup grant.
    Pickup,
    /// Inventory only.
    InventoryOnly,
    /// Individual only.
    IndividualOnly,
    /// Forbidden.
    Forbidden,
}

/// Item kind data (`Q2ItemDefinition` kind fields).
#[derive(Debug, Clone)]
pub enum Q2ItemKindData {
    /// Ammo.
    Ammo {
        /// Pickup quantity.
        quantity: f64,
        /// Capacity.
        capacity: f64,
        /// Whether weapon ammo.
        weapon_ammo: bool,
        /// Infinite-ammo quantity override: absent selects 1000 in
        /// infinite-ammo deathmatch, null selects the normal quantity.
        infinite_ammo_quantity: Option<Option<f64>>,
    },
    /// Weapon.
    Weapon {
        /// Ammo item.
        ammo: Option<ItemId>,
        /// Cooperative stay override (defaults to true).
        coop_stay: Option<bool>,
    },
    /// Health.
    Health {
        /// Amount.
        amount: f64,
        /// Whether the maximum is ignored.
        ignore_maximum: bool,
        /// Whether timed (mega health).
        timed: bool,
    },
    /// Armor.
    Armor {
        /// Points.
        points: f64,
        /// Maximum.
        maximum: f64,
        /// Normal protection.
        normal: f64,
        /// Energy protection.
        energy: f64,
    },
    /// Armor shard.
    Shard,
    /// Powerup.
    Power {
        /// Cooperative stay.
        coop_stay: bool,
    },
    /// Power armor.
    PowerArmor {
        /// Armor kind.
        armor: Q2PowerArmorKind,
    },
    /// Maximum health.
    MaximumHealth {
        /// Increase.
        increase: f64,
        /// Whether health is filled.
        fill: bool,
    },
    /// Key.
    Key,
    /// Ammo pack.
    AmmoPack {
        /// Whether full.
        full: bool,
    },
    /// Custom item.
    Custom {
        /// Capacity.
        capacity: f64,
        /// Quantity.
        quantity: f64,
        /// Cooperative stay.
        coop_stay: bool,
        /// Whether droppable.
        droppable: bool,
        /// Pickup callback.
        pickup: Q2CustomPickup,
        /// Use callback.
        use_item: Option<Q2CustomUse>,
    },
}

/// Item definition (`Q2ItemDefinition`).
#[derive(Debug, Clone)]
pub struct Q2ItemDefinition {
    /// Classname.
    pub classname: String,
    /// Model path.
    pub model: String,
    /// Icon.
    pub icon: String,
    /// Display name.
    pub name: String,
    /// Pickup sound.
    pub sound: String,
    /// Whether the model rotates.
    pub rotate: bool,
    /// Respawn seconds.
    pub respawn: f64,
    /// Console give policy.
    pub console_give: Option<Q2ConsoleGive>,
    /// Kind data.
    pub kind: Q2ItemKindData,
}

impl Q2ItemDefinition {
    /// Item kind discriminator.
    pub fn kind(&self) -> Q2ItemKind {
        match self.kind {
            Q2ItemKindData::Ammo { .. } => Q2ItemKind::Ammo,
            Q2ItemKindData::Weapon { .. } => Q2ItemKind::Weapon,
            Q2ItemKindData::Health { .. } => Q2ItemKind::Health,
            Q2ItemKindData::Armor { .. } => Q2ItemKind::Armor,
            Q2ItemKindData::Shard => Q2ItemKind::Shard,
            Q2ItemKindData::Power { .. } => Q2ItemKind::Power,
            Q2ItemKindData::PowerArmor { .. } => Q2ItemKind::PowerArmor,
            Q2ItemKindData::MaximumHealth { .. } => Q2ItemKind::MaximumHealth,
            Q2ItemKindData::Key => Q2ItemKind::Key,
            Q2ItemKindData::AmmoPack { .. } => Q2ItemKind::AmmoPack,
            Q2ItemKindData::Custom { .. } => Q2ItemKind::Custom,
        }
    }
}

/// Power armor kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PowerArmorKind {
    /// None.
    None,
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Item hooks (`Q2ItemHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2ItemHooks {
    /// Weapon picked.
    pub weapon_picked: fn(ActorId, ItemId, bool),
    /// Silencer charges; other powerups use expiry seconds.
    pub silencer: fn(ActorId, f64),
    /// Power armor state.
    pub power_armor: fn(ActorId, Q2PowerArmorKind),
    /// Ammo pack pickup.
    pub ammo_pack: Option<fn(OwnedActor, &mut Q2GameServices, bool)>,
    /// Random respawn replacement.
    pub random_respawn: Option<fn(ActorId, &mut Q2GameServices) -> Option<ActorId>>,
    /// Weapon respawn seconds.
    pub weapon_respawn_seconds: Option<fn() -> f64>,
}

/// Pickup policy (`Q2PickupPolicy`).
#[derive(Debug, Clone, Copy)]
pub struct Q2PickupPolicy {
    /// Instanced cooperative pickups.
    pub instanced_coop: Option<fn(&mut Q2GameServices) -> bool>,
    /// Whether a pickup can be taken.
    pub can_pickup: fn(ActorId, &mut Q2GameServices, ActorId) -> bool,
    /// Whether a pickup attempt proceeds.
    pub before_pickup: fn(ActorId, &mut Q2GameServices, ActorId) -> bool,
    /// Before targets fire.
    pub before_targets: Option<fn(ActorId, &mut Q2GameServices, ActorId, bool)>,
    /// After a pickup attempt.
    pub after_pickup: fn(ActorId, &mut Q2GameServices, ActorId, bool),
    /// Whether the pickup stays after being taken.
    pub keep_after_pickup: fn(ActorId, &mut Q2GameServices, ActorId) -> bool,
}

/// Player powerups (`Q2PlayerPowerups`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q2PlayerPowerups {
    /// Quad expiry.
    pub quad_until: f64,
    /// Invulnerability expiry.
    pub invulnerability_until: f64,
    /// Breather expiry.
    pub breather_until: f64,
    /// Enviro expiry.
    pub enviro_until: f64,
}

/// Pickup state (`PickupState`).
#[derive(Debug, Clone)]
pub struct PickupState {
    /// Item definition.
    pub item: Q2ItemDefinition,
    /// Whether targets fired.
    pub targets_used: bool,
    /// Whether retained after pickup.
    pub retained: bool,
    /// Expiry time.
    pub expires_at: Option<f64>,
}

/// Inventory item (`Q2InventoryItem`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2InventoryItem {
    /// Item id.
    pub id: ItemId,
    /// Classname.
    pub classname: String,
    /// Display name.
    pub name: String,
    /// Item kind.
    pub kind: Q2ItemKind,
    /// Quantity.
    pub quantity: f64,
    /// Whether usable.
    pub usable: bool,
    /// Whether droppable.
    pub droppable: bool,
    /// Cooperative stay.
    pub stay_coop: bool,
    /// Capacity.
    pub capacity: f64,
    /// Whether a weapon.
    pub weapon: bool,
    /// Console give policy.
    pub console_give: Q2ConsoleGive,
}

/// Drop options (`Q2DropOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q2DropOptions {
    /// Whether touch is immediate.
    pub immediate_touch: bool,
    /// Whether dropped on player death.
    pub player_death: bool,
    /// Yaw offset.
    pub yaw_offset: Option<f64>,
    /// Expiry time.
    pub expires_at: Option<f64>,
}

/// Pickup checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PickupCheckpoint {
    /// Pickup actor.
    pub actor: SavedActorId,
    /// Item classname.
    pub classname: String,
    /// Whether targets fired.
    pub targets_used: bool,
    /// Whether retained.
    pub retained: bool,
    /// Expiry time.
    pub expires_at: Option<f64>,
}

/// Powerup checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PowerCheckpoint {
    /// Player actor.
    pub actor: SavedActorId,
    /// Powerup state.
    pub state: Q2PlayerPowerups,
}

/// Items checkpoint (`Q2ItemsCheckpoint`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2ItemsCheckpoint {
    /// Power cube count.
    pub power_cube_count: i32,
    /// Pickup entries.
    pub pickups: Vec<Q2PickupCheckpoint>,
    /// Powerup entries.
    pub powers: Vec<Q2PowerCheckpoint>,
    /// Power armor bindings.
    pub power_armor_bindings: Vec<SavedActorId>,
}

/// Arena runtime state for this module.
pub struct ItemRuntime {
    /// Item catalog by classname.
    pub catalog: HashMap<String, Q2ItemDefinition>,
    /// Pickup states by actor.
    pub pickups: HashMap<ActorId, PickupState>,
    /// Player powerups by actor.
    pub powers: HashMap<ActorId, Q2PlayerPowerups>,
    /// Power armor bindings by actor.
    pub power_armor_bindings: HashSet<ActorId>,
    /// Shared player power-armor cell stores.
    pub power_cells: HashMap<ActorId, Rc<RefCell<f64>>>,
    /// Power cube count.
    pub power_cube_count: i32,
    /// Item hooks.
    pub hooks: Option<Q2ItemHooks>,
    /// Pickup policy.
    pub pickup_policy: Option<Q2PickupPolicy>,
    /// Pickup admission.
    pub pickup_admission: Option<Box<dyn PickupAdmission>>,
}

impl Default for ItemRuntime {
    fn default() -> Self {
        ItemRuntime {
            catalog: base_item_catalog(),
            pickups: HashMap::new(),
            powers: HashMap::new(),
            power_armor_bindings: HashSet::new(),
            power_cells: HashMap::new(),
            power_cube_count: 0,
            hooks: None,
            pickup_policy: None,
            pickup_admission: None,
        }
    }
}

impl ItemRuntime {
    /// Clean arena state after any actor release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.pickups.remove(actor);
        self.powers.remove(actor);
        self.power_armor_bindings.remove(actor);
        self.power_cells.remove(actor);
    }
}

/// Q2 item module (`Q2ItemModule`).
#[derive(Debug, Clone, Copy)]
pub struct Q2ItemModule {
    /// Item hooks.
    hooks: Q2ItemHooks,
}

/// Create the Q2 item module (`createQ2ItemModule`).
pub fn create_q2_item_module(hooks: Q2ItemHooks) -> Q2ItemModule {
    Q2ItemModule { hooks }
}

/// Item id (`q2:${classname}`).
fn item_id(item: &Q2ItemDefinition) -> ItemId {
    format!("q2:{}", item.classname)
}

/// Whether an entity is a dropped pickup (`isDropped`).
fn is_dropped(game: &Q2GameServices, actor: &ActorId) -> bool {
    game.require_entity(actor).spawnflags & 0x30000 != 0
}

/// Whether an item stays in cooperative mode (`staysCoop`).
fn stays_coop(item: &Q2ItemDefinition) -> bool {
    match &item.kind {
        Q2ItemKindData::Key => true,
        Q2ItemKindData::Weapon { coop_stay, .. } => coop_stay.unwrap_or(true),
        Q2ItemKindData::Power { coop_stay } => *coop_stay,
        Q2ItemKindData::Custom { coop_stay, .. } => *coop_stay,
        _ => false,
    }
}

/// Base weapon display name and icon (`weaponNames`).
fn base_weapon_names(name: Q2BaseWeaponName) -> (&'static str, &'static str) {
    match name {
        Q2BaseWeaponName::Blaster => ("Blaster", "w_blaster"),
        Q2BaseWeaponName::Shotgun => ("Shotgun", "w_shotgun"),
        Q2BaseWeaponName::Supershotgun => ("Super Shotgun", "w_sshotgun"),
        Q2BaseWeaponName::Machinegun => ("Machinegun", "w_machinegun"),
        Q2BaseWeaponName::Chaingun => ("Chaingun", "w_chaingun"),
        Q2BaseWeaponName::Grenades => ("Grenades", "a_grenades"),
        Q2BaseWeaponName::Grenadelauncher => ("Grenade Launcher", "w_glauncher"),
        Q2BaseWeaponName::Rocketlauncher => ("Rocket Launcher", "w_rlauncher"),
        Q2BaseWeaponName::Hyperblaster => ("HyperBlaster", "w_hyperblaster"),
        Q2BaseWeaponName::Railgun => ("Railgun", "w_railgun"),
        Q2BaseWeaponName::Bfg => ("BFG10K", "w_bfg"),
    }
}

/// Base ammunition definitions (`ammunition`).
fn base_ammunition() -> Vec<Q2ItemDefinition> {
    let ammo = |classname: &str,
                model: &str,
                icon: &str,
                name: &str,
                rotate: bool,
                quantity: f64,
                capacity: f64,
                weapon_ammo: bool| {
        Q2ItemDefinition {
            classname: classname.to_string(),
            model: model.to_string(),
            icon: icon.to_string(),
            name: name.to_string(),
            sound: "misc/am_pkup.wav".to_string(),
            rotate,
            respawn: 30.0,
            console_give: None,
            kind: Q2ItemKindData::Ammo {
                quantity,
                capacity,
                weapon_ammo,
                infinite_ammo_quantity: None,
            },
        }
    };
    vec![
        ammo("ammo_shells", "models/items/ammo/shells/medium/tris.md2", "a_shells", "Shells", false, 10.0, 100.0, false),
        ammo("ammo_bullets", "models/items/ammo/bullets/medium/tris.md2", "a_bullets", "Bullets", false, 50.0, 200.0, false),
        ammo("ammo_cells", "models/items/ammo/cells/medium/tris.md2", "a_cells", "Cells", false, 50.0, 200.0, false),
        ammo("ammo_rockets", "models/items/ammo/rockets/medium/tris.md2", "a_rockets", "Rockets", false, 5.0, 50.0, false),
        ammo("ammo_slugs", "models/items/ammo/slugs/medium/tris.md2", "a_slugs", "Slugs", false, 10.0, 50.0, false),
        ammo("ammo_grenades", "models/items/ammo/grenades/medium/tris.md2", "a_grenades", "Grenades", true, 5.0, 50.0, true),
    ]
}

/// Base key definitions (`keys`).
fn base_keys() -> Vec<Q2ItemDefinition> {
    let key = |classname: &str, model: &str, icon: &str, name: &str| Q2ItemDefinition {
        classname: classname.to_string(),
        model: model.to_string(),
        icon: icon.to_string(),
        name: name.to_string(),
        sound: "items/pkup.wav".to_string(),
        rotate: classname != "key_commander_head",
        respawn: 0.0,
        console_give: None,
        kind: Q2ItemKindData::Key,
    };
    vec![
        key("key_data_cd", "models/items/keys/data_cd/tris.md2", "k_datacd", "Data CD"),
        key("key_power_cube", "models/items/keys/power/tris.md2", "k_powercube", "Power Cube"),
        key("key_pyramid", "models/items/keys/pyramid/tris.md2", "k_pyramid", "Pyramid Key"),
        key("key_data_spinner", "models/items/keys/spinner/tris.md2", "k_dataspin", "Data Spinner"),
        key("key_pass", "models/items/keys/pass/tris.md2", "k_security", "Security Pass"),
        key("key_blue_key", "models/items/keys/key/tris.md2", "k_bluekey", "Blue Key"),
        key("key_red_key", "models/items/keys/red_key/tris.md2", "k_redkey", "Red Key"),
        key("key_commander_head", "models/monsters/commandr/head/tris.md2", "k_comhead", "Commander's Head"),
        key("key_airstrike_target", "models/items/keys/target/tris.md2", "i_airstrike", "Airstrike Marker"),
    ]
}

/// Base item catalog (`items`).
fn base_item_catalog() -> HashMap<String, Q2ItemDefinition> {
    let mut items = base_ammunition();
    items.extend(base_keys());
    let visual = |kind: Q2ItemKindData,
                  classname: &str,
                  model: &str,
                  icon: &str,
                  name: &str,
                  sound: &str,
                  rotate: bool,
                  respawn: f64| {
        Q2ItemDefinition {
            classname: classname.to_string(),
            model: model.to_string(),
            icon: icon.to_string(),
            name: name.to_string(),
            sound: sound.to_string(),
            rotate,
            respawn,
            console_give: None,
            kind,
        }
    };
    items.extend([
        visual(Q2ItemKindData::AmmoPack { full: false }, "item_bandolier", "models/items/band/tris.md2", "p_bandolier", "Bandolier", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::AmmoPack { full: true }, "item_pack", "models/items/pack/tris.md2", "i_pack", "Ammo Pack", "items/pkup.wav", true, 180.0),
        visual(Q2ItemKindData::Health { amount: 10.0, ignore_maximum: false, timed: false }, "item_health", "models/items/healing/medium/tris.md2", "i_health", "Health", "items/n_health.wav", false, 30.0),
        visual(Q2ItemKindData::Health { amount: 2.0, ignore_maximum: true, timed: false }, "item_health_small", "models/items/healing/stimpack/tris.md2", "i_health", "Health", "items/s_health.wav", false, 30.0),
        visual(Q2ItemKindData::Health { amount: 25.0, ignore_maximum: false, timed: false }, "item_health_large", "models/items/healing/large/tris.md2", "i_health", "Health", "items/l_health.wav", false, 30.0),
        visual(Q2ItemKindData::Health { amount: 100.0, ignore_maximum: true, timed: true }, "item_health_mega", "models/items/mega_h/tris.md2", "i_health", "Health", "items/m_health.wav", false, 20.0),
        visual(Q2ItemKindData::Armor { points: 25.0, maximum: 50.0, normal: 0.3, energy: 0.0 }, "item_armor_jacket", "models/items/armor/jacket/tris.md2", "i_jacketarmor", "Jacket Armor", "misc/ar1_pkup.wav", true, 20.0),
        visual(Q2ItemKindData::Armor { points: 50.0, maximum: 100.0, normal: 0.6, energy: 0.3 }, "item_armor_combat", "models/items/armor/combat/tris.md2", "i_combatarmor", "Combat Armor", "misc/ar1_pkup.wav", true, 20.0),
        visual(Q2ItemKindData::Armor { points: 100.0, maximum: 200.0, normal: 0.8, energy: 0.6 }, "item_armor_body", "models/items/armor/body/tris.md2", "i_bodyarmor", "Body Armor", "misc/ar1_pkup.wav", true, 20.0),
        visual(Q2ItemKindData::Shard, "item_armor_shard", "models/items/armor/shard/tris.md2", "i_jacketarmor", "Armor Shard", "misc/ar2_pkup.wav", true, 20.0),
        visual(Q2ItemKindData::Power { coop_stay: false }, "item_quad", "models/items/quaddama/tris.md2", "p_quad", "Quad Damage", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::Power { coop_stay: false }, "item_invulnerability", "models/items/invulner/tris.md2", "p_invulnerability", "Invulnerability", "items/pkup.wav", true, 300.0),
        visual(Q2ItemKindData::Power { coop_stay: false }, "item_silencer", "models/items/silencer/tris.md2", "p_silencer", "Silencer", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::Power { coop_stay: true }, "item_breather", "models/items/breather/tris.md2", "p_rebreather", "Rebreather", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::Power { coop_stay: true }, "item_enviro", "models/items/enviro/tris.md2", "p_envirosuit", "Environment Suit", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::PowerArmor { armor: Q2PowerArmorKind::Screen }, "item_power_screen", "models/items/armor/screen/tris.md2", "i_powerscreen", "Power Screen", "misc/ar3_pkup.wav", true, 60.0),
        visual(Q2ItemKindData::PowerArmor { armor: Q2PowerArmorKind::Shield }, "item_power_shield", "models/items/armor/shield/tris.md2", "i_powershield", "Power Shield", "misc/ar3_pkup.wav", true, 60.0),
        visual(Q2ItemKindData::MaximumHealth { increase: 1.0, fill: true }, "item_adrenaline", "models/items/adrenal/tris.md2", "p_adrenaline", "Adrenaline", "items/pkup.wav", true, 60.0),
        visual(Q2ItemKindData::MaximumHealth { increase: 2.0, fill: false }, "item_ancient_head", "models/items/c_head/tris.md2", "i_fixme", "Ancient Head", "items/pkup.wav", true, 60.0),
    ]);
    for weapon in base_weapons() {
        if weapon.name == Q2BaseWeaponName::Grenades {
            continue;
        }
        let (name, icon) = base_weapon_names(weapon.name);
        items.push(Q2ItemDefinition {
            classname: weapon.definition.classname.clone(),
            model: weapon.definition.world_model.clone(),
            icon: icon.to_string(),
            name: name.to_string(),
            sound: "misc/w_pkup.wav".to_string(),
            rotate: true,
            respawn: 30.0,
            console_give: None,
            kind: Q2ItemKindData::Weapon { ammo: weapon.definition.ammo.clone(), coop_stay: None },
        });
    }
    items.into_iter().map(|item| (item.classname.clone(), item)).collect()
}

/// Weapon inventory entries (`weaponInventory`).
fn weapon_inventory_entries(
    catalog: &HashMap<String, Q2ItemDefinition>,
) -> Vec<(ItemId, f64)> {
    let mut entries = Vec::new();
    let mut classnames: Vec<&String> = catalog.keys().collect();
    classnames.sort();
    for classname in classnames {
        let item = &catalog[classname];
        match &item.kind {
            Q2ItemKindData::Ammo { capacity, .. } => entries.push((item_id(item), *capacity)),
            Q2ItemKindData::Weapon { .. } => entries.push((item_id(item), 32767.0)),
            _ => {}
        }
    }
    entries
}

/// Ensure an admitted inventory entry (`ensure`).
fn ensure_inventory_entry(game: &mut Q2GameServices, actor: &OwnedActor, item: &str, capacity: f64) {
    if !game.host.inventory().entries(actor.id()).iter().any(|entry| entry.item == item) {
        game.host.inventory().configure(
            actor,
            &InventoryEntry { item: item.to_string(), count: 0.0, capacity, count_policy: None },
        );
    }
}

/// Read pickup state, panicking when absent (`pickup`).
fn pickup_state(game: &Q2GameServices, actor: &ActorId) -> PickupState {
    game.items.pickups.get(actor).cloned().unwrap_or_else(|| panic!("Q2 item callback has no pickup state"))
}

/// Read item hooks, panicking when unregistered.
fn item_hooks(game: &Q2GameServices) -> Q2ItemHooks {
    game.items.hooks.expect("Q2 item hooks are not registered")
}

/// Power cell inventory item.
const POWER_CELL_ITEM: &str = "q2:ammo_cells";

/// Read the player power cell count, flushing combat drains first.
pub fn player_power_cells(game: &mut Q2GameServices, actor: &ActorId) -> f64 {
    flush_player_power_cells(game, actor);
    game.host.inventory().count(actor, &POWER_CELL_ITEM.to_string())
}

/// Flush combat-drained power cells back to inventory.
pub fn flush_player_power_cells(game: &mut Q2GameServices, actor: &ActorId) {
    let Some(cells) = game.items.power_cells.get(actor).cloned() else {
        return;
    };
    let count = *cells.borrow();
    let owned = game.owned_of(actor.clone());
    let Some(entry) = game.host.inventory().entries(actor).into_iter().find(|entry| entry.item == POWER_CELL_ITEM) else {
        panic!("Q2 power armor requires its admitted cell inventory");
    };
    game.host.inventory().configure(&owned, &InventoryEntry { count, ..entry });
}

/// Apply an inventory delta to the shared power cell store.
pub fn add_player_power_cells(game: &mut Q2GameServices, actor: &ActorId, delta: f64) {
    if delta == 0.0 {
        return;
    }
    if let Some(cells) = game.items.power_cells.get(actor) {
        let next = *cells.borrow() + delta;
        *cells.borrow_mut() = next;
    }
}

/// Set the shared power cell store after an absolute configure.
pub fn set_player_power_cells(game: &mut Q2GameServices, actor: &ActorId, count: f64) {
    if let Some(cells) = game.items.power_cells.get(actor) {
        *cells.borrow_mut() = count;
    }
}

/// Ammo pickup quantity (`ammoQuantity`).
fn ammo_quantity(game: &Q2GameServices, actor: &ActorId, item: &Q2ItemDefinition) -> f64 {
    let Q2ItemKindData::Ammo { quantity, weapon_ammo, infinite_ammo_quantity, .. } = &item.kind
    else {
        panic!("Q2 ammo quantity needs an ammo descriptor");
    };
    if *weapon_ammo
        && *infinite_ammo_quantity != Some(None)
        && game.deathmatch_flags() & 8192 != 0
    {
        return infinite_ammo_quantity.unwrap_or(Some(1000.0)).unwrap_or(1000.0);
    }
    let count = game.require_entity(actor).count;
    if count == 0 { *quantity } else { f64::from(count) }
}

/// Weapon ammo grants (`weaponAmmo`).
fn weapon_ammo_grants(
    game: &Q2GameServices,
    actor: &ActorId,
    item: &Q2ItemDefinition,
) -> Vec<PickupAmmoGrant> {
    let Q2ItemKindData::Weapon { ammo, .. } = &item.kind else {
        panic!("Q2 weapon grants need a weapon descriptor");
    };
    if game.require_entity(actor).spawnflags & 0x10000 != 0 || ammo.is_none() {
        return Vec::new();
    }
    let ammo_id = ammo.as_ref().expect("weapon ammo");
    let Some(descriptor) = ammo_id
        .strip_prefix("q2:")
        .and_then(|classname| game.items.catalog.get(classname))
    else {
        return Vec::new();
    };
    if let Q2ItemKindData::Ammo { quantity, .. } = &descriptor.kind {
        let amount = if game.deathmatch_flags() & 8192 != 0 { 1000.0 } else { *quantity };
        vec![PickupAmmoGrant { item: item_id(descriptor), amount }]
    } else {
        Vec::new()
    }
}

/// Owned weapon count (`weaponOwned`).
fn weapon_owned(game: &mut Q2GameServices, player: &ActorId, item: &Q2ItemDefinition) -> f64 {
    let id = item_id(item);
    match &game.items.pickup_admission {
        Some(admission) if admission.maps(PickupMapKind::Weapons, &id) => {
            if admission.owns(player, &id) { 1.0 } else { 0.0 }
        }
        _ => game.host.inventory().count(player, &id),
    }
}

/// Weapon eligibility (`weaponEligible`).
fn weapon_eligible(
    game: &mut Q2GameServices,
    actor: &ActorId,
    item: &Q2ItemDefinition,
    previous: f64,
) -> bool {
    let id = item_id(item);
    if let Some(admission) = &game.items.pickup_admission {
        if admission.maps(PickupMapKind::Weapons, &id) && id == "q2:weapon_blaster" {
            return false;
        }
    }
    let instanced = game
        .items
        .pickup_policy
        .and_then(|policy| policy.instanced_coop)
        .is_some_and(|instanced| instanced(game));
    let stays = if game.options.mode == Q2Mode::Coop {
        !instanced
    } else {
        game.options.mode == Q2Mode::Deathmatch && game.deathmatch_flags() & 4 != 0
    };
    !stays || previous <= 0.0 || is_dropped(game, actor)
}

/// Whether a non-owner can touch a temporary pickup (`ownerCanTouch`).
fn owner_can_touch(game: &Q2GameServices, actor: &ActorId, player: &ActorId) -> bool {
    game.require_entity(actor).owner.as_ref() != Some(player)
}

/// Grant an item (`grant`).
fn grant_item(
    this: ActorId,
    game: &mut Q2GameServices,
    player: &OwnedActor,
    item: &Q2ItemDefinition,
) -> bool {
    let Some(current) = game.host.combat().read(player.id()) else {
        return false;
    };
    let player_entity = game.entity(player.id()).cloned();
    let maximum = player_entity.map(|entity| entity.max_health).unwrap_or(0.0);
    let maximum = if maximum == 0.0 { 100.0 } else { maximum };
    match &item.kind {
        Q2ItemKindData::Custom { capacity, pickup, .. } => {
            ensure_inventory_entry(game, player, &item_id(item), *capacity);
            if !pickup(this, game, player.clone()) {
                return false;
            }
        }
        Q2ItemKindData::Key => {
            ensure_inventory_entry(game, player, &item_id(item), 32767.0);
            if game.options.mode == Q2Mode::Coop {
                if item.classname == "key_power_cube"
                    || game.options.edition == Q2Edition::Rerelease
                        && item.classname == "key_explosive_charges"
                {
                    let Some(player_entity) = game.entity(player.id()).cloned() else {
                        panic!("Q2 cooperative key pickup requires admitted source player fields");
                    };
                    let cubes = (game.require_entity(&this).spawnflags & 0xff00) >> 8;
                    if player_entity.power_cubes & cubes != 0 {
                        return false;
                    }
                    game.require_entity_mut(player.id()).power_cubes |= cubes;
                } else if game.host.inventory().count(player.id(), &item_id(item)) != 0.0 {
                    return false;
                }
            }
            game.host.inventory().give(player, &item_id(item), 1.0);
            return true;
        }
        Q2ItemKindData::AmmoPack { full } => {
            for ammo in base_ammunition() {
                let Q2ItemKindData::Ammo { quantity, capacity, .. } = &ammo.kind else {
                    continue;
                };
                let pack_capacity = if ammo.classname == "ammo_bullets" || ammo.classname == "ammo_cells" {
                    if *full { 300.0 } else { 250.0 }
                } else if ammo.classname == "ammo_shells" {
                    if *full { 200.0 } else { 150.0 }
                } else if ammo.classname == "ammo_slugs" {
                    if *full { 100.0 } else { 75.0 }
                } else if *full {
                    100.0
                } else {
                    *capacity
                };
                ensure_inventory_entry(game, player, &item_id(&ammo), *capacity);
                let Some(entry) = game
                    .host
                    .inventory()
                    .entries(player.id())
                    .into_iter()
                    .find(|entry| entry.item == item_id(&ammo))
                else {
                    panic!("Q2 ammo pack has no admitted ammo inventory");
                };
                game.host.inventory().configure(
                    player,
                    &InventoryEntry {
                        capacity: entry.capacity.max(pack_capacity),
                        ..entry
                    },
                );
                if *full || ammo.classname == "ammo_bullets" || ammo.classname == "ammo_shells" {
                    let given = game.host.inventory().give(player, &item_id(&ammo), *quantity);
                    if item_id(&ammo) == POWER_CELL_ITEM {
                        add_player_power_cells(game, player.id(), given);
                    }
                }
            }
            if let Some(ammo_pack) = item_hooks(game).ammo_pack {
                ammo_pack(player.clone(), game, *full);
            }
        }
        Q2ItemKindData::Ammo { capacity, weapon_ammo, .. } => {
            let quantity = ammo_quantity(game, &this, item);
            let mapped = game
                .items
                .pickup_admission
                .as_ref()
                .is_some_and(|admission| admission.maps(PickupMapKind::Ammo, &item_id(item)));
            if mapped {
                let admission = game.items.pickup_admission.as_ref().expect("ammo admission");
                let taken = if *weapon_ammo {
                    admission.ammo_weapon(
                        player,
                        &PickupAmmoWeaponGrant {
                            item: item_id(item),
                            amount: quantity,
                            weapon: item_id(item),
                        },
                        AmmoWeaponSelection { mode: PickupSelection::Always, when: AmmoWeaponTiming::EmptyAmmo },
                    )
                } else {
                    admission.ammo(player, &PickupAmmoGrant { item: item_id(item), amount: quantity }, false)
                };
                if !taken {
                    return false;
                }
            } else {
                ensure_inventory_entry(game, player, &item_id(item), *capacity);
                let old = game.host.inventory().count(player.id(), &item_id(item));
                let given = game.host.inventory().give(player, &item_id(item), quantity);
                if given == 0.0 {
                    return false;
                }
                if item_id(item) == POWER_CELL_ITEM {
                    add_player_power_cells(game, player.id(), given);
                }
                if *weapon_ammo && old == 0.0 {
                    (item_hooks(game).weapon_picked)(player.id().clone(), item_id(item), true);
                }
            }
        }
        Q2ItemKindData::Weapon { .. } => {
            let id = item_id(item);
            let mapped = game
                .items
                .pickup_admission
                .as_ref()
                .is_some_and(|admission| admission.maps(PickupMapKind::Weapons, &id));
            if mapped && id == "q2:weapon_blaster" {
                return false;
            }
            if !mapped {
                ensure_inventory_entry(game, player, &id, 32767.0);
            }
            let previous = weapon_owned(game, player.id(), item);
            if !weapon_eligible(game, &this, item, previous) {
                return false;
            }
            if !mapped {
                game.host.inventory().give(player, &id, 1.0);
            }
            let grants = weapon_ammo_grants(game, &this, item);
            if !mapped {
                for grant in &grants {
                    let ammo = grant.item.strip_prefix("q2:").and_then(|classname| game.items.catalog.get(classname));
                    let Some(ammo) = ammo else {
                        panic!("Weapon grant has no source ammo descriptor");
                    };
                    let Q2ItemKindData::Ammo { capacity, .. } = &ammo.kind else {
                        panic!("Weapon grant has no source ammo descriptor");
                    };
                    ensure_inventory_entry(game, player, &grant.item, *capacity);
                    let given = game.host.inventory().give(player, &grant.item, grant.amount);
                    if grant.item == POWER_CELL_ITEM {
                        add_player_power_cells(game, player.id(), given);
                    }
                }
                (item_hooks(game).weapon_picked)(player.id().clone(), id, previous == 0.0);
            } else {
                let admission = game.items.pickup_admission.as_ref().expect("weapon admission");
                if !admission.weapon(
                    player,
                    &PickupWeaponGrant { item: id, ammo: grants },
                    if previous == 0.0 { PickupSelection::Always } else { PickupSelection::Never },
                ) {
                    return false;
                }
            }
        }
        Q2ItemKindData::Health { amount, ignore_maximum, .. } => {
            if !ignore_maximum && current.health >= maximum {
                return false;
            }
            let count = game.require_entity(&this).count;
            let amount = if count == 0 { *amount } else { f64::from(count) };
            game.host.combat().set_health(
                player,
                if *ignore_maximum { current.health + amount } else { maximum.min(current.health + amount) },
            );
        }
        Q2ItemKindData::Armor { .. } | Q2ItemKindData::Shard => {
            let Some(next) = pickup_q2_armor(item, &current.armor.regular) else {
                return false;
            };
            game.host.combat().set_regular_armor(player, &next);
        }
        Q2ItemKindData::MaximumHealth { increase, fill } => {
            let increase = if *fill && game.options.mode == Q2Mode::Deathmatch { 0.0 } else { *increase };
            if game.entity(player.id()).is_some() {
                game.require_entity_mut(player.id()).max_health = maximum + increase;
            }
            if *fill && current.health < maximum + increase {
                game.host.combat().set_health(player, maximum + increase);
            }
        }
        Q2ItemKindData::Power { coop_stay } => {
            ensure_inventory_entry(game, player, &item_id(item), 32767.0);
            let quantity = game.host.inventory().count(player.id(), &item_id(item));
            let instanced = game
                .items
                .pickup_policy
                .and_then(|policy| policy.instanced_coop)
                .is_some_and(|instanced| instanced(game));
            if game.options.edition == Q2Edition::Rerelease && game.options.skill == 0 && quantity >= 3.0
                || game.options.skill == 1 && quantity >= 2.0
                || game.options.skill >= 2 && quantity >= 1.0
                || game.options.mode == Q2Mode::Coop && !instanced && *coop_stay && quantity > 0.0
            {
                return false;
            }
            game.host.inventory().give(player, &item_id(item), 1.0);
            if game.options.mode == Q2Mode::Deathmatch
                && (game.deathmatch_flags() & 16 != 0
                    || item.classname == "item_quad" && game.require_entity(&this).spawnflags & 0x20000 != 0)
            {
                let expires = pickup_state(game, &this).expires_at;
                let duration = expires.map(|expires| 0.0f64.max(expires - game.host.now())).unwrap_or(30.0);
                use_inventory_item_at(player.clone(), &item_id(item), game, duration);
            }
        }
        Q2ItemKindData::PowerArmor { .. } => {
            ensure_inventory_entry(game, player, &item_id(item), 32767.0);
            let old = game.host.inventory().count(player.id(), &item_id(item));
            game.host.inventory().give(player, &item_id(item), 1.0);
            if game.options.mode == Q2Mode::Deathmatch && old == 0.0 {
                use_inventory_item_at(player.clone(), &item_id(item), game, 30.0);
            }
        }
    }
    true
}

/// Finish a grant (`finishGrant`).
fn finish_grant(
    this: ActorId,
    game: &mut Q2GameServices,
    player: &OwnedActor,
    item: &Q2ItemDefinition,
) {
    if item.kind() == Q2ItemKind::Key {
        return;
    }
    if item.kind() == Q2ItemKind::Weapon
        && !is_dropped(game, &this)
        && (game.options.mode == Q2Mode::Coop
            || game.options.mode == Q2Mode::Deathmatch && game.deathmatch_flags() & 4 != 0)
    {
        game.items.pickups.get_mut(&this).expect("pickup state").retained = true;
        return;
    }
    if let Q2ItemKindData::Health { timed: true, .. } = &item.kind {
        game.require_entity_mut(&this).owner = Some(player.id().clone());
        game.items.pickups.get_mut(&this).expect("pickup state").retained = true;
        game.require_entity_mut(&this).visible = false;
        game.set_solid(this.clone(), Q2Solid::None);
        if !game.host.actors().is_live(&this) {
            return;
        }
        game.show(this.clone());
        if game.host.actors().is_live(&this) {
            game.schedule(this, 5.0, mega_health);
        }
        return;
    }
    let respawn = if item.kind() == Q2ItemKind::Weapon {
        item_hooks(game).weapon_respawn_seconds.map(|seconds| seconds()).unwrap_or(item.respawn)
    } else {
        item.respawn
    };
    if !is_dropped(game, &this)
        && game.options.mode == Q2Mode::Deathmatch
        && game.host.actors().is_live(&this)
    {
        set_respawn(this, game, respawn);
    }
}

/// Pick up armor (`pickupQ2Armor`).
fn pickup_q2_armor(item: &Q2ItemDefinition, old: &RegularArmorState) -> Option<RegularArmorState> {
    if matches!(old, RegularArmorState::Source { .. }) {
        return None;
    }
    if item.kind() == Q2ItemKind::Shard {
        if let RegularArmorState::Q2 { points, .. } = old {
            if *points > 0.0 {
                if let RegularArmorState::Q2 { points, normal_protection, energy_protection, item } = old.clone() {
                    return Some(RegularArmorState::Q2 {
                        points: points + 2.0,
                        normal_protection,
                        energy_protection,
                        item,
                    });
                }
            }
        }
        return Some(RegularArmorState::Q2 {
            item: "q2:item_armor_jacket".to_string(),
            points: 2.0,
            normal_protection: 0.3,
            energy_protection: 0.0,
        });
    }
    let Q2ItemKindData::Armor { points, maximum, normal, energy } = &item.kind else {
        panic!("Q2 armor pickup needs armor");
    };
    let (old_points, old_normal, old_energy, old_item) = match old {
        RegularArmorState::Q2 { points, normal_protection, energy_protection, item } => {
            (points, normal_protection, energy_protection, item)
        }
        _ => {
            return Some(RegularArmorState::Q2 {
                item: item_id(item),
                points: *points,
                normal_protection: *normal,
                energy_protection: *energy,
            });
        }
    };
    if *old_points == 0.0 {
        return Some(RegularArmorState::Q2 {
            item: item_id(item),
            points: *points,
            normal_protection: *normal,
            energy_protection: *energy,
        });
    }
    if *normal > *old_normal {
        return Some(RegularArmorState::Q2 {
            item: item_id(item),
            points: maximum.min(points + (old_points * old_normal / normal).trunc()),
            normal_protection: *normal,
            energy_protection: *energy,
        });
    }
    let maximum: f64 = if old_item == "q2:item_armor_jacket" {
        50.0
    } else if old_item == "q2:item_armor_combat" {
        100.0
    } else {
        200.0
    };
    let points = maximum.min(old_points + (points * normal / old_normal).trunc());
    if points <= *old_points {
        return None;
    }
    let _ = old_energy;
    if let RegularArmorState::Q2 { normal_protection, energy_protection, item, .. } = old.clone() {
        Some(RegularArmorState::Q2 { points, normal_protection, energy_protection, item })
    } else {
        None
    }
}

/// Item touch continuation (`touch` closures).
struct ItemTouch {
    /// Pickup actor.
    pickup: ActorId,
    /// Player actor.
    player: ActorId,
    /// Resolved owner.
    owner: OwnedActor,
    /// Item definition.
    item: Q2ItemDefinition,
    /// Whether the original grant ran.
    original_ran: bool,
}

impl ItemTouch {
    /// Whether the pickup and owner are still live (`live`).
    fn live(&self, game: &mut Q2GameServices) -> bool {
        game.entity(&self.pickup).is_some()
            && game.host.actors().resolve_owned(&self.player).as_ref() == Some(&self.owner)
    }
}

impl Q2OriginalPickupContinuation for ItemTouch {
    fn eligible(&mut self, game: &mut Q2GameServices) -> bool {
        let before = game
            .items
            .pickup_policy
            .map(|policy| policy.before_pickup)
            .map(|before| before(self.pickup.clone(), game, self.player.clone()))
            .unwrap_or(true);
        before && self.live(game)
    }

    fn original(&mut self, game: &mut Q2GameServices) -> bool {
        self.original_ran = true;
        game.items.pickups.get_mut(&self.pickup).expect("pickup state").retained = false;
        grant_item(self.pickup.clone(), game, &self.owner, &self.item.clone())
    }

    fn complete(&mut self, game: &mut Q2GameServices, taken: bool) {
        if !self.live(game) {
            return;
        }
        if taken {
            if !self.original_ran {
                game.items.pickups.get_mut(&self.pickup).expect("pickup state").retained = false;
            }
            let item = self.item.clone();
            finish_grant(self.pickup.clone(), game, &self.owner, &item);
            if !self.live(game) {
                return;
            }
            game.host_emit(Q2PresentationEvent::Pickup {
                player: self.player.clone(),
                item: item_id(&item),
                icon: item.icon.clone(),
                name: item.name.clone(),
            });
            if !self.live(game) {
                return;
            }
            if let Some(body) = game.host.bodies().read(&self.player) {
                game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                    actor: Some(self.player.clone()),
                    origin: body.origin,
                    path: item.sound.clone(),
                    channel: 3,
                    volume: 1.0,
                    attenuation: 1.0,
                    reliable: false,
                    loop_: Q2SoundLoop::Once,
                    loop_owner: None,
                }));
            }
            if !self.live(game) {
                return;
            }
        }
        if let Some(before_targets) =
            game.items.pickup_policy.and_then(|policy| policy.before_targets)
        {
            before_targets(self.pickup.clone(), game, self.player.clone(), taken);
        }
        if !self.live(game) {
            return;
        }
        // Source items fire targets on the first attempted pickup, even when full.
        if !pickup_state(game, &self.pickup).targets_used {
            let authored = game.require_entity(&self.pickup).authored_target();
            game.use_targets(&authored, Some(&self.player), false);
            if let Some(state) = game.items.pickups.get_mut(&self.pickup) {
                state.targets_used = true;
            }
        }
        if !self.live(game) {
            return;
        }
        if let Some(policy) = game.items.pickup_policy {
            (policy.after_pickup)(self.pickup.clone(), game, self.player.clone(), taken);
        }
        if !taken || !self.live(game) {
            return;
        }
        let stays = game.options.mode == Q2Mode::Coop && stays_coop(&self.item);
        let keep = game
            .items
            .pickup_policy
            .map(|policy| (policy.keep_after_pickup)(self.pickup.clone(), game, self.player.clone()))
            .unwrap_or(false);
        if (!stays || is_dropped(game, &self.pickup))
            && !pickup_state(game, &self.pickup).retained
            && !keep
            && self.live(game)
        {
            game.remove_actor(self.pickup.clone());
        }
    }
}

/// Touch a pickup (`touch`).
fn touch_item_at(pickup: ActorId, game: &mut Q2GameServices, player: ActorId) {
    if !game.host.is_player(&player) {
        return;
    }
    let health = game.host.combat().read(&player).map(|state| state.health).unwrap_or(0.0);
    if health < 1.0 {
        return;
    }
    let item = pickup_state(game, &pickup).item.clone();
    let Some(owner) = game.host.actors().resolve_owned(&player) else {
        return;
    };
    let mut touch = ItemTouch { pickup: pickup.clone(), player: player.clone(), owner, item: item.clone(), original_ran: false };
    if game.host.original_pickups().is_some() {
        let default_resource = match item.kind() {
            Q2ItemKind::Armor | Q2ItemKind::Shard => {
                Some(PickupResource::Protection { channel: ProtectionChannel::Regular })
            }
            Q2ItemKind::PowerArmor => {
                Some(PickupResource::Protection { channel: ProtectionChannel::Powered })
            }
            Q2ItemKind::Ammo | Q2ItemKind::Weapon | Q2ItemKind::Key => {
                Some(PickupResource::Inventory { item: item_id(&item) })
            }
            _ => None,
        };
        let count = game.require_entity(&pickup).count;
        let source = game.require_entity(&pickup).actor.owner().clone();
        let offer = OriginalPickupOffer {
            recipient: player,
            pickup: pickup.clone(),
            source,
            item: item_id(&item),
            default_resource,
            count: if count == 0 {
                PickupCount::Default
            } else {
                PickupCount::Override { amount: f64::from(count) }
            },
            dropped: is_dropped(game, &pickup),
            time: SourceTime::Seconds(game.host.now() as f32),
            cargo: Vec::new(),
            grant: None,
        };
        let admission = game.host.original_pickups().expect("original admission");
        admission.touch(&offer, &mut touch);
        return;
    }
    if touch.eligible(game) {
        let taken = touch.original(game);
        touch.complete(game, taken);
    }
}

/// Use an inventory item (`use`).
fn use_inventory_item_at(
    player: OwnedActor,
    item_id: &str,
    game: &mut Q2GameServices,
    duration: f64,
) -> bool {
    let Some(item) = item_id
        .strip_prefix("q2:")
        .and_then(|classname| game.items.catalog.get(classname))
        .cloned()
    else {
        return false;
    };
    if game.host.inventory().count(player.id(), &item_id.to_string()) == 0.0 {
        return false;
    }
    match &item.kind {
        Q2ItemKindData::Custom { use_item, .. } => {
            return use_item.map(|use_item| use_item(player, game)).unwrap_or(false);
        }
        Q2ItemKindData::PowerArmor { armor } => {
            let armor_state = game.host.combat().read(player.id()).map(|state| state.armor);
            let active = matches!(armor_state, Some(armor) if !matches!(armor.powered, PoweredProtectionState::None));
            if !active && player_power_cells(game, player.id()) == 0.0 {
                return false;
            }
            let cells = player_power_cells(game, player.id());
            game.host.combat().set_powered_protection(
                &player,
                &if active {
                    PoweredProtectionState::None
                } else if *armor == Q2PowerArmorKind::Shield {
                    PoweredProtectionState::Shield { cells }
                } else {
                    PoweredProtectionState::Screen { cells }
                },
            );
            (item_hooks(game).power_armor)(
                player.id().clone(),
                if active { Q2PowerArmorKind::None } else { *armor },
            );
            return true;
        }
        Q2ItemKindData::Power { .. } => {}
        _ => return false,
    }
    if !game.host.inventory().consume(&player, &item_id.to_string(), 1.0) {
        return false;
    }
    let now = game.host.now();
    let state = game.items.powers.entry(player.id().clone()).or_default();
    match item.classname.as_str() {
        "item_quad" => state.quad_until = state.quad_until.max(now) + duration,
        "item_silencer" => (item_hooks(game).silencer)(player.id().clone(), 30.0),
        "item_breather" => state.breather_until = state.breather_until.max(now) + 30.0,
        "item_enviro" => state.enviro_until = state.enviro_until.max(now) + 30.0,
        "item_invulnerability" => {
            state.invulnerability_until = state.invulnerability_until.max(now) + 30.0;
            let until = state.invulnerability_until;
            game.host.combat().set_traits(
                &player,
                &CombatTraitChanges { invulnerable: Some(true), ..CombatTraitChanges::default() },
            );
            let timer = game.create("invulnerability_expiry", BTreeMap::new());
            game.require_entity_mut(&timer).owner = Some(player.id().clone());
            game.schedule(timer, until - now, invulnerability_expiry);
        }
        _ => {}
    }
    true
}

/// Drop a source pickup (`dropSource`).
fn drop_source(
    actor: &OwnedActor,
    game: &mut Q2GameServices,
    item: &Q2ItemDefinition,
    options: &Q2DropOptions,
    count: f64,
) -> ActorId {
    let Some(body) = game.host.bodies().read(actor.id()) else {
        panic!("Item drop owner has no shared body");
    };
    let dropped = game.create(&item.classname, BTreeMap::new());
    game.require_entity_mut(&dropped).model = item.model.clone();
    game.require_entity_mut(&dropped).owner = Some(actor.id().clone());
    game.require_entity_mut(&dropped).spawnflags = if options.player_death { 0x20000 } else { 0x10000 };
    game.require_entity_mut(&dropped).effects = if item.rotate { 1 } else { 0 };
    game.require_entity_mut(&dropped).render_flags = 512 | 0x8000;
    game.require_entity_mut(&dropped).count = count as i32;
    game.items.pickups.insert(
        dropped.clone(),
        PickupState { item: item.clone(), targets_used: false, retained: false, expires_at: options.expires_at },
    );
    let view = game
        .host
        .player_view_state(actor.id())
        .map(|view| view.view_angles)
        .unwrap_or(body.angles);
    let forward = movedir(vec3(view.x, view.y + options.yaw_offset.unwrap_or(0.0) as f32, view.z));
    let bounds = qa_core::math::Bounds {
        min: vec3(-15.0, -15.0, -15.0),
        max: vec3(15.0, 15.0, 15.0),
    };
    let origin = if game.host.is_player(actor.id()) {
        let end = add3(
            add3(body.origin, scale3(forward, 24.0)),
            vec3(0.0, 0.0, -16.0),
        );
        game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end,
            bounds: Some(bounds),
            ignore: Some(actor.id().clone()),
            mask: 1,
            exclude: Vec::new(),
        }).end
    } else {
        body.origin
    };
    let mut moved = game.body_of(dropped.clone());
    moved.origin = origin;
    moved.bounds = qa_core::math::Bounds {
        min: vec3(-15.0, -15.0, -15.0),
        max: vec3(15.0, 15.0, 15.0),
    };
    let velocity = scale3(forward, 100.0);
    moved.velocity = vec3(velocity.x, velocity.y, 300.0);
    game.write_body(dropped.clone(), &moved, false);
    game.require_entity_mut(&dropped).touch = Some(temporary_touch);
    game.set_solid(dropped.clone(), Q2Solid::Trigger);
    game.set_motion_kind(dropped.clone(), Q2MotionKind::Toss);
    game.show(dropped.clone());
    if options.immediate_touch {
        make_touchable(dropped.clone(), game);
    } else {
        game.schedule(dropped.clone(), 1.0, make_touchable);
    }
    dropped
}

/// Drop an item (`drop`).
fn drop_item_at(
    this: ActorId,
    game: &mut Q2GameServices,
    item_id: &str,
    options: &Q2DropOptions,
) -> Option<ActorId> {
    let item = item_id.strip_prefix("q2:").and_then(|classname| game.items.catalog.get(classname)).cloned();
    let descriptor = lookup_item(game, item_id);
    let (Some(item), Some(descriptor)) = (item, descriptor) else {
        return None;
    };
    if !descriptor.droppable {
        return None;
    }
    if game.options.mode == Q2Mode::Coop
        && !game
            .items
            .pickup_policy
            .and_then(|policy| policy.instanced_coop)
            .is_some_and(|instanced| instanced(game))
        && descriptor.stay_coop
    {
        return None;
    }
    if item_id == POWER_CELL_ITEM {
        flush_player_power_cells(game, &this);
    }
    let count = game.host.inventory().count(&this, &item_id.to_string());
    let owned = game.owned_of(this);
    let drop_count = match &item.kind {
        Q2ItemKindData::Ammo { quantity, .. } => count.min(*quantity),
        _ => 0.0,
    };
    Some(drop_source(&owned, game, &item, options, drop_count))
}

/// Spawn an item (`spawnItem`).
fn spawn_item_at(game: &mut Q2GameServices, actor: ActorId, descriptor: &str) -> bool {
    let Some(item) = game.items.catalog.get(descriptor).cloned() else {
        return false;
    };
    let flags = game.deathmatch_flags();
    if game.options.mode == Q2Mode::Deathmatch
        && ((flags & 1 != 0 && (item.kind() == Q2ItemKind::Health || item.kind() == Q2ItemKind::MaximumHealth))
            || (flags & 2 != 0 && item.kind() == Q2ItemKind::Power)
            || (flags & 2048 != 0
                && (item.kind() == Q2ItemKind::Armor
                    || item.kind() == Q2ItemKind::Shard
                    || item.kind() == Q2ItemKind::PowerArmor))
            || (flags & 8192 != 0
                && ((item.kind() == Q2ItemKind::Ammo
                    && !matches!(&item.kind, Q2ItemKindData::Ammo { weapon_ammo: true, .. }))
                    || item.classname == "weapon_bfg")))
    {
        game.remove_actor(actor);
        return true;
    }
    game.items.pickups.insert(
        actor.clone(),
        PickupState { item: item.clone(), targets_used: false, retained: false, expires_at: None },
    );
    if game.options.mode == Q2Mode::Coop
        && (item.classname == "key_power_cube"
            || game.options.edition == Q2Edition::Rerelease && item.classname == "key_explosive_charges")
    {
        let bit = game.items.power_cube_count;
        game.items.power_cube_count += 1;
        game.require_entity_mut(&actor).spawnflags |= 1 << (8 + bit);
    }
    if game.require_entity(&actor).model.is_empty() {
        game.require_entity_mut(&actor).model = item.model.clone();
    }
    if item.rotate {
        game.require_entity_mut(&actor).effects |= 1;
    }
    game.require_entity_mut(&actor).render_flags |= 512;
    if item.classname == "key_commander_head" {
        game.require_entity_mut(&actor).effects |= 2;
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, 2.0 * frame_seconds, drop_to_floor);
    true
}

/// Observe pickup supply (`observeSupply`).
#[allow(unpredictable_function_pointer_comparisons)]
fn observe_supply_at(
    game: &mut Q2GameServices,
    pickup: ActorId,
    recipient: ActorId,
) -> Option<PickupSupplyObservation> {
    let entity = game.entity(&pickup)?.clone();
    let item = game.items.pickups.get(&pickup)?.item.clone();
    let offer = match &item.kind {
        Q2ItemKindData::Ammo { weapon_ammo, .. } => {
            let amount = ammo_quantity(game, &pickup, &item);
            if *weapon_ammo {
                PickupSupplyOffer::AmmoWeapon(PickupAmmoWeaponGrant {
                    item: item_id(&item),
                    amount,
                    weapon: item_id(&item),
                })
            } else {
                PickupSupplyOffer::Ammo(PickupAmmoGrant { item: item_id(&item), amount })
            }
        }
        Q2ItemKindData::Weapon { .. } => PickupSupplyOffer::Weapon(PickupWeaponGrant {
            item: item_id(&item),
            ammo: weapon_ammo_grants(game, &pickup, &item),
        }),
        _ => return None,
    };
    let inactive = || PickupSupplyObservation {
        actor: pickup.clone(),
        offer: offer.clone(),
        availability: PickupAvailability::Inactive,
    };
    if !entity.visible || entity.solid != Q2Solid::Trigger {
        if entity.spawn.values.contains_key("team") || item_hooks(game).random_respawn.is_some() {
            return Some(inactive());
        }
        return Some(if entity.think == Some(respawn_item) && entity.next_think.is_some() {
            PickupSupplyObservation {
                actor: pickup,
                offer,
                availability: PickupAvailability::Respawning {
                    at_seconds: entity.next_think.expect("respawn time"),
                },
            }
        } else {
            inactive()
        });
    }
    if entity.touch != Some(touch_pickup) && entity.touch != Some(temporary_touch) {
        return Some(inactive());
    }
    let healthy = game.host.combat().read(&recipient).map(|state| state.health).unwrap_or(0.0) >= 1.0;
    let can_pickup = game
        .items
        .pickup_policy
        .map(|policy| (policy.can_pickup)(pickup.clone(), game, recipient.clone()))
        .unwrap_or(true);
    let owned = if item.kind() == Q2ItemKind::Weapon {
        weapon_owned(game, &recipient, &item)
    } else {
        0.0
    };
    let weapon_ok = item.kind() != Q2ItemKind::Weapon || weapon_eligible(game, &pickup, &item, owned);
    let eligible = game.host.is_player(&recipient)
        && game.host.actors().is_live(&recipient)
        && healthy
        && (entity.touch != Some(temporary_touch) || owner_can_touch(game, &pickup, &recipient))
        && can_pickup
        && weapon_ok;
    Some(PickupSupplyObservation {
        actor: pickup,
        offer,
        availability: PickupAvailability::Ready { eligible },
    })
}

/// Preview pickup supply (`previewSupply`).
fn preview_supply_at(
    game: &mut Q2GameServices,
    pickup: ActorId,
    recipient: ActorId,
) -> Option<PickupSupplyPreview> {
    let observation = observe_supply_at(game, pickup, recipient.clone())?;
    let offer = observation.offer.clone();
    if game.items.pickup_admission.is_some() {
        if let PickupSupplyOffer::Weapon(grant) = &offer {
            if grant.item == "q2:weapon_blaster" {
                return Some(PickupSupplyPreview { accepted: false, ammo: Vec::new(), weapons: Vec::new() });
            }
        }
        let admission = game.items.pickup_admission.as_ref().expect("supply admission");
        return Some(admission.preview(&recipient, &offer));
    }
    let mut inventory: Vec<InventoryEntry> = game.host.inventory().entries(&recipient);
    let ammo: Vec<PickupAmmoGrant> = match &offer {
        PickupSupplyOffer::Weapon(grant) => grant.ammo.clone(),
        PickupSupplyOffer::Ammo(grant) => vec![grant.clone()],
        PickupSupplyOffer::AmmoWeapon(grant) => {
            vec![PickupAmmoGrant { item: grant.item.clone(), amount: grant.amount }]
        }
    };
    for grant in &ammo {
        let descriptor = grant.item.strip_prefix("q2:").and_then(|classname| game.items.catalog.get(classname));
        let Some(descriptor) = descriptor else {
            panic!("Weapon grant has no source ammo descriptor");
        };
        let Q2ItemKindData::Ammo { capacity, .. } = &descriptor.kind else {
            panic!("Weapon grant has no source ammo descriptor");
        };
        if !inventory.iter().any(|entry| entry.item == grant.item) {
            inventory.push(InventoryEntry { item: grant.item.clone(), count: 0.0, capacity: *capacity, count_policy: None });
        }
    }
    if let PickupSupplyOffer::Weapon(grant) = &offer {
        if !inventory.iter().any(|entry| entry.item == grant.item) {
            inventory.push(InventoryEntry {
                item: grant.item.clone(),
                count: 0.0,
                capacity: 32767.0,
                count_policy: None,
            });
        }
        return Some(preview_pickup_grants(
            &inventory,
            &PickupGrantPlan::Weapon {
                weapons: vec![PickupAmmoGrant { item: grant.item.clone(), amount: 1.0 }],
                ammo,
            },
        ));
    }
    let shared = match &offer {
        PickupSupplyOffer::AmmoWeapon(grant) => vec![grant.weapon.clone()],
        _ => Vec::new(),
    };
    Some(preview_pickup_grants(
        &inventory,
        &PickupGrantPlan::Ammo {
            acceptance: PickupAcceptance::Nonzero,
            ammo,
            weapons: PickupWeaponLink::SharedAmmo { items: shared },
        },
    ))
}

/// Respawn a pickup (`respawn`).
fn respawn_item(this: ActorId, game: &mut Q2GameServices) {
    let team = game.require_entity(&this).spawn.values.get("team").cloned();
    let mut candidates = Vec::new();
    if team.is_none() {
        candidates.push(this.clone());
    } else {
        let mut member = game.require_entity(&this).team_master.clone();
        while let Some(next) = member {
            let Some(entity) = game.entity(&next).cloned() else {
                break;
            };
            member = entity.chain.clone();
            candidates.push(next);
        }
    }
    let index = (game.host.random() * candidates.len() as f64).floor() as usize;
    let mut selected = candidates.get(index).cloned().unwrap_or_else(|| this.clone());
    if game.options.edition == Q2Edition::Classic {
        if let Some(random_respawn) = item_hooks(game).random_respawn {
            let replacement = random_respawn(selected.clone(), game);
            if let Some(replacement) = replacement {
                if replacement != selected {
                    game.remove_actor(selected);
                    selected = replacement;
                }
            }
        }
    }
    if !game.host.actors().is_live(&selected) {
        return;
    }
    game.require_entity_mut(&selected).visible = true;
    game.set_solid(selected.clone(), Q2Solid::Trigger);
    game.show(selected.clone());
    let origin = game.body_of(selected.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:item-respawn".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    if game.options.edition == Q2Edition::Rerelease {
        if let Some(random_respawn) = item_hooks(game).random_respawn {
            random_respawn(selected, game);
        }
    }
}

/// Schedule a respawn (`setRespawn`).
fn set_respawn(this: ActorId, game: &mut Q2GameServices, seconds: f64) {
    game.items.pickups.get_mut(&this).expect("pickup state").retained = true;
    game.require_entity_mut(&this).visible = false;
    game.set_solid(this.clone(), Q2Solid::None);
    if !game.host.actors().is_live(&this) {
        return;
    }
    game.show(this.clone());
    if !game.host.actors().is_live(&this) {
        return;
    }
    game.schedule(this, seconds, respawn_item);
}

/// Drop a pickup to the floor (`dropToFloor`).
fn drop_to_floor(this: ActorId, game: &mut Q2GameServices) {
    let mut moved = game.body_of(this.clone());
    moved.bounds.min = vec3(-15.0, -15.0, -15.0);
    moved.bounds.max = vec3(15.0, 15.0, 15.0);
    game.write_body(this.clone(), &moved, false);
    let origin = game.body_of(this.clone()).origin;
    let bounds = game.body_of(this.clone()).bounds;
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: add3(origin, vec3(0.0, 0.0, -128.0)),
        bounds: Some(bounds),
        ignore: Some(this.clone()),
        mask: 3,
        exclude: Vec::new(),
    });
    if trace.start_solid {
        let classname = game.require_entity(&this).classname.clone();
        game.host.diagnostic(&format!("Q2 {classname} starts solid"));
        game.remove_actor(this);
        return;
    }
    let mut moved = game.body_of(this.clone());
    moved.origin = trace.end;
    game.write_body(this.clone(), &moved, true);
    game.require_entity_mut(&this).touch = Some(touch_pickup);
    game.set_solid(this.clone(), Q2Solid::Trigger);
    game.set_motion_kind(this.clone(), Q2MotionKind::Toss);
    if game.require_entity(&this).spawn.values.contains_key("team") {
        game.require_entity_mut(&this).flags &= !0x400;
        let team_chain = game.require_entity(&this).team_chain.clone();
        game.require_entity_mut(&this).chain = team_chain;
        game.require_entity_mut(&this).team_chain = None;
        game.require_entity_mut(&this).visible = false;
        game.set_solid(this.clone(), Q2Solid::None);
        if game.require_entity(&this).team_master.as_ref() == Some(&this) {
            let frame_seconds = game.host.frame_seconds();
            game.schedule(this.clone(), frame_seconds, respawn_item);
        }
    }
    if game.require_entity(&this).spawnflags & 2 != 0 {
        game.require_entity_mut(&this).touch = None;
        game.require_entity_mut(&this).effects &= !1;
        game.require_entity_mut(&this).render_flags &= !512;
        game.set_solid(this.clone(), Q2Solid::Box);
    }
    if game.require_entity(&this).spawnflags & 1 != 0 {
        game.require_entity_mut(&this).visible = false;
        game.set_solid(this.clone(), Q2Solid::None);
        game.require_entity_mut(&this).use_ = Some(use_item_callback);
    }
    game.show(this);
}

/// Mega health think (`megaHealth`).
fn mega_health(this: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&this).owner.clone();
    let resolved = owner.as_ref().and_then(|owner| game.host.actors().resolve_owned(owner));
    let health = resolved.as_ref().and_then(|owner| game.host.combat().read(owner.id()));
    let maximum = owner
        .as_ref()
        .and_then(|owner| game.entity(owner))
        .map(|entity| entity.max_health)
        .unwrap_or(0.0);
    let maximum = if maximum == 0.0 { 100.0 } else { maximum };
    if let (Some(owner), Some(health)) = (resolved, health) {
        if health.health > maximum {
            game.host.combat().set_health(&owner, health.health - 1.0);
            game.schedule(this, 1.0, mega_health);
            return;
        }
    }
    if !is_dropped(game, &this) && game.options.mode == Q2Mode::Deathmatch {
        set_respawn(this, game, 20.0);
    } else {
        game.remove_actor(this);
    }
}

/// Item touch (`touchPickup`).
fn touch_pickup(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    touch_item_at(this, game, contact.other);
}

/// Temporary touch (`temporaryTouch`).
fn temporary_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if owner_can_touch(game, &this, &contact.other) {
        touch_item_at(this, game, contact.other);
    }
}

/// Item use (`useItem`).
fn use_item_callback(
    this: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    game.require_entity_mut(&this).visible = true;
    game.require_entity_mut(&this).use_ = None;
    let boxed = game.require_entity(&this).spawnflags & 2 != 0;
    game.set_solid(this.clone(), if boxed { Q2Solid::Box } else { Q2Solid::Trigger });
    game.show(this);
}

/// Make a dropped pickup touchable (`makeTouchable`).
fn make_touchable(this: ActorId, game: &mut Q2GameServices) {
    game.require_entity_mut(&this).touch = Some(touch_pickup);
    let expires = pickup_state(game, &this).expires_at;
    if let Some(expires) = expires {
        let delay = 0.0f64.max(expires - game.host.now());
        game.schedule(this, delay, free_q2_entity);
    } else if game.options.mode == Q2Mode::Deathmatch {
        game.schedule(this, 29.0, free_q2_entity);
    }
}

/// Invulnerability expiry (`invulnerabilityExpiry`).
fn invulnerability_expiry(this: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&this).owner.clone();
    let player = owner.as_ref().and_then(|owner| game.host.actors().resolve_owned(owner));
    if let Some(player) = player {
        let until = game.items.powers.get(player.id()).map(|powers| powers.invulnerability_until).unwrap_or(0.0);
        if game.host.now() >= until {
            game.host.combat().set_traits(
                &player,
                &CombatTraitChanges { invulnerable: Some(false), ..CombatTraitChanges::default() },
            );
        }
    }
    game.remove_actor(this);
}

/// List inventory items (`list`).
fn list_items(game: &Q2GameServices) -> Vec<Q2InventoryItem> {
    let mut classnames: Vec<&String> = game.items.catalog.keys().collect();
    classnames.sort();
    classnames
        .into_iter()
        .map(|classname| {
            let item = &game.items.catalog[classname];
            let (quantity, capacity) = match &item.kind {
                Q2ItemKindData::Ammo { quantity, capacity, .. } => (*quantity, *capacity),
                Q2ItemKindData::Custom { quantity, capacity, .. } => (*quantity, *capacity),
                _ => (1.0, 32767.0),
            };
            let usable = match &item.kind {
                Q2ItemKindData::Custom { use_item, .. } => use_item.is_some(),
                Q2ItemKindData::Power { .. }
                | Q2ItemKindData::PowerArmor { .. }
                | Q2ItemKindData::Weapon { .. } => true,
                Q2ItemKindData::Ammo { weapon_ammo, .. } => *weapon_ammo,
                _ => false,
            };
            let droppable = match &item.kind {
                Q2ItemKindData::Custom { droppable, .. } => *droppable,
                Q2ItemKindData::Key
                | Q2ItemKindData::Ammo { .. }
                | Q2ItemKindData::Power { .. }
                | Q2ItemKindData::PowerArmor { .. } => true,
                Q2ItemKindData::Weapon { .. } => item.classname != "weapon_blaster",
                _ => false,
            };
            let weapon = matches!(&item.kind, Q2ItemKindData::Weapon { .. })
                || matches!(&item.kind, Q2ItemKindData::Ammo { weapon_ammo: true, .. });
            Q2InventoryItem {
                id: item_id(item),
                classname: item.classname.clone(),
                name: item.name.clone(),
                kind: item.kind(),
                quantity,
                usable,
                droppable,
                stay_coop: stays_coop(item),
                capacity,
                weapon,
                console_give: item.console_give.unwrap_or(Q2ConsoleGive::Pickup),
            }
        })
        .collect()
}

/// Look up an inventory item (`lookup`).
fn lookup_item(game: &Q2GameServices, value: &str) -> Option<Q2InventoryItem> {
    let key = value.to_lowercase();
    list_items(game).into_iter().find(|item| {
        item.id.to_lowercase() == key
            || item.classname.to_lowercase() == key
            || item.name.to_lowercase() == key
    })
}

/// Give starting items (`giveStartItems`).
fn give_start_items_at(player: &OwnedActor, game: &mut Q2GameServices, expression: &str) {
    let grants: Vec<(Q2ItemDefinition, i64)> = parse_q2_start_items(expression)
        .into_iter()
        .map(|grant| {
            let item = game.items.catalog.get(&grant.classname.to_lowercase()).cloned();
            match item {
                Some(item) if item.console_give != Some(Q2ConsoleGive::InventoryOnly) => {
                    (item, grant.count)
                }
                _ => panic!("Invalid Q2 starting item: {}", grant.classname),
            }
        })
        .collect();
    for (item, count) in grants {
        if count == 0 {
            let id = item_id(&item);
            let mut destinations = vec![id.clone()];
            let mapped_ammo = matches!(&item.kind, Q2ItemKindData::Ammo { .. })
                && game
                    .items
                    .pickup_admission
                    .as_ref()
                    .is_some_and(|admission| admission.maps(PickupMapKind::Ammo, &id));
            let mapped_weapon = matches!(&item.kind, Q2ItemKindData::Weapon { .. })
                && id != "q2:weapon_blaster"
                && game
                    .items
                    .pickup_admission
                    .as_ref()
                    .is_some_and(|admission| admission.maps(PickupMapKind::Weapons, &id));
            if mapped_ammo {
                let admission = game.items.pickup_admission.as_ref().expect("start admission");
                destinations = admission
                    .preview(
                        player.id(),
                        &PickupSupplyOffer::Ammo(PickupAmmoGrant { item: id.clone(), amount: 0.0 }),
                    )
                    .ammo
                    .into_iter()
                    .map(|receipt| receipt.item)
                    .collect();
                if matches!(&item.kind, Q2ItemKindData::Ammo { weapon_ammo: true, .. }) {
                    destinations.extend(
                        admission
                            .preview(
                                player.id(),
                                &PickupSupplyOffer::Weapon(PickupWeaponGrant {
                                    item: id.clone(),
                                    ammo: Vec::new(),
                                }),
                            )
                            .weapons
                            .into_iter()
                            .map(|receipt| receipt.item),
                    );
                }
            } else if mapped_weapon {
                let admission = game.items.pickup_admission.as_ref().expect("start admission");
                destinations = admission
                    .preview(
                        player.id(),
                        &PickupSupplyOffer::Weapon(PickupWeaponGrant { item: id, ammo: Vec::new() }),
                    )
                    .weapons
                    .into_iter()
                    .map(|receipt| receipt.item)
                    .collect();
            }
            let mut seen = HashSet::new();
            for destination in destinations {
                if !seen.insert(destination.clone()) {
                    continue;
                }
                if let Some(entry) = game
                    .host
                    .inventory()
                    .entries(player.id())
                    .into_iter()
                    .find(|entry| entry.item == destination)
                {
                    game.host.inventory().configure(
                        player,
                        &InventoryEntry { count: 0.0, ..entry },
                    );
                    if destination == POWER_CELL_ITEM {
                        set_player_power_cells(game, player.id(), 0.0);
                    }
                }
            }
            continue;
        }
        let temporary = game.create(&item.classname, BTreeMap::new());
        game.require_entity_mut(&temporary).count =
            count.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        game.require_entity_mut(&temporary).spawnflags |= 0x10000;
        game.items.pickups.insert(
            temporary.clone(),
            PickupState { item: item.clone(), targets_used: false, retained: false, expires_at: None },
        );
        if grant_item(temporary.clone(), game, player, &item) {
            finish_grant(temporary.clone(), game, player, &item);
        }
        if game.host.actors().is_live(&temporary) {
            game.remove_actor(temporary);
        }
    }
}

/// Give an ammo count (`giveAmmoCount`).
fn give_ammo_count_at(
    player: &OwnedActor,
    game: &mut Q2GameServices,
    item_id: &str,
    count: Option<f64>,
) {
    let Some(item) = lookup_item(game, item_id) else {
        panic!("Console ammo grant requires a source ammo item");
    };
    if item.kind != Q2ItemKind::Ammo {
        panic!("Console ammo grant requires a source ammo item");
    }
    let mapped = game
        .items
        .pickup_admission
        .as_ref()
        .is_some_and(|admission| admission.maps(PickupMapKind::Ammo, &item.id));
    let destinations = if mapped {
        let admission = game.items.pickup_admission.as_ref().expect("ammo admission");
        admission
            .preview(
                player.id(),
                &PickupSupplyOffer::Ammo(PickupAmmoGrant { item: item.id.clone(), amount: 0.0 }),
            )
            .ammo
            .into_iter()
            .map(|receipt| receipt.item)
            .collect()
    } else {
        if item.id == POWER_CELL_ITEM {
            flush_player_power_cells(game, player.id());
        }
        vec![item.id.clone()]
    };
    for destination in destinations {
        let entry = game
            .host
            .inventory()
            .entries(player.id())
            .into_iter()
            .find(|entry| entry.item == destination);
        if mapped && entry.is_none() {
            panic!("Console ammo destination was not admitted");
        }
        let next = count.unwrap_or_else(|| entry.as_ref().map(|entry| entry.count).unwrap_or(0.0) + item.quantity);
        if mapped && entry.is_some() {
            let entry = entry.expect("ammo entry");
            let source_counter = matches!(
                entry.count_policy,
                Some(InventoryCountPolicy::SourceCounter(_))
            );
            game.host.inventory().configure(
                player,
                &InventoryEntry { count: if source_counter { next } else { 0.0f64.max(next) }, ..entry },
            );
        } else {
            game.host.inventory().configure(
                player,
                &InventoryEntry {
                    item: destination.clone(),
                    count: next,
                    capacity: entry.as_ref().map(|entry| entry.capacity).unwrap_or(item.capacity),
                    count_policy: Some(InventoryCountPolicy::SourceCounter(
                        SourceCounterArithmetic::Int32,
                    )),
                },
            );
        }
        if destination == POWER_CELL_ITEM {
            set_player_power_cells(game, player.id(), next);
        }
    }
}

/// Replace a pickup item (`replaceItem`).
fn replace_item_at(game: &mut Q2GameServices, actor: ActorId, descriptor: &str) {
    let Some(item) = game.items.catalog.get(descriptor).cloned() else {
        panic!("Q2 replacement item is not registered: {descriptor}");
    };
    let state = pickup_state(game, &actor);
    game.items.pickups.insert(
        actor.clone(),
        PickupState { item: item.clone(), ..state },
    );
    game.require_entity_mut(&actor).classname = item.classname.clone();
    game.require_entity_mut(&actor).model = item.model.clone();
    game.require_entity_mut(&actor).effects = if item.rotate { 1 } else { 0 };
    game.show(actor);
}

/// Configure a player (`configurePlayer`).
fn configure_player_at(actor: &OwnedActor, game: &mut Q2GameServices, give_blaster: bool) {
    let weapons: HashMap<ItemId, f64> =
        weapon_inventory_entries(&game.items.catalog.clone()).into_iter().collect();
    let catalog = game.items.catalog.clone();
    for item in catalog.values() {
        match item.kind() {
            Q2ItemKind::Ammo
            | Q2ItemKind::Weapon
            | Q2ItemKind::Power
            | Q2ItemKind::PowerArmor
            | Q2ItemKind::Key
            | Q2ItemKind::Custom => {}
            _ => continue,
        }
        let fallback = match &item.kind {
            Q2ItemKindData::Custom { capacity, .. } => *capacity,
            _ => 32767.0,
        };
        ensure_inventory_entry(game, actor, &item_id(item), weapons.get(&item_id(item)).copied().unwrap_or(fallback));
    }
    if give_blaster && game.host.inventory().count(actor.id(), &"q2:weapon_blaster".to_string()) == 0.0 {
        game.host.inventory().give(actor, &"q2:weapon_blaster".to_string(), 1.0);
    }
    bind_power_armor_at(actor, game);
}

/// Bind power armor cells (`bindPowerArmor`).
fn bind_power_armor_at(actor: &OwnedActor, game: &mut Q2GameServices) {
    if game.items.power_armor_bindings.contains(actor.id()) {
        return;
    }
    let cells = Rc::new(RefCell::new(
        game.host.inventory().count(actor.id(), &POWER_CELL_ITEM.to_string()),
    ));
    game.items.power_cells.insert(actor.id().clone(), cells.clone());
    game.host.combat().bind_power_armor_cells(actor, Box::new(SharedPowerCells(cells)));
    game.items.power_armor_bindings.insert(actor.id().clone());
}

/// Item spawn dispatch (`spawn`).
fn spawn_item_entity(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    spawn_item_at(game, actor, &classname)
}

/// Item module item-name handler (unused; dispatch reads the live catalog).
fn item_module_name(_classname: &str) -> Option<String> {
    None
}

impl Q2ItemModule {
    /// Register hooks and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.items.hooks = Some(self.hooks);
        let mut callbacks = Q2CallbackDefinitions::default();
        callbacks.think.insert("q2_items_respawn", respawn_item);
        callbacks.think.insert("q2_items_drop_to_floor", drop_to_floor);
        callbacks.think.insert("q2_items_make_touchable", make_touchable);
        callbacks.think.insert("q2_items_mega_health", mega_health);
        callbacks.think.insert("q2_items_invulnerability_expiry", invulnerability_expiry);
        callbacks.touch.insert("Touch_Item", touch_pickup);
        callbacks.touch.insert("drop_temp_touch", temporary_touch);
        callbacks.use_.insert("Use_Item", use_item_callback);
        let spawn: Q2SpawnFn = spawn_item_entity;
        let item_name: Q2ItemNameFn = item_module_name;
        SpawnModule { spawn, item_name, callbacks }
    }

    /// Register an item (`register`).
    pub fn register_item(&self, game: &mut Q2GameServices, item: Q2ItemDefinition) {
        if game.items.catalog.contains_key(&item.classname) {
            panic!("Q2 item already registered: {}", item.classname);
        }
        game.items.catalog.insert(item.classname.clone(), item);
    }

    /// Resolve an item display name (`itemName`).
    pub fn item_name(&self, game: &Q2GameServices, classname: &str) -> Option<String> {
        game.items.catalog.get(classname).map(|item| item.name.clone())
    }

    /// Model and sound resource paths (`resourcePaths`).
    pub fn resource_paths(
        &self,
        game: &Q2GameServices,
        classnames: &HashSet<String>,
    ) -> Vec<String> {
        game.items
            .catalog
            .values()
            .filter(|item| classnames.contains(&item.classname))
            .flat_map(|item| [item.model.clone(), format!("sound/{}", item.sound)])
            .collect()
    }

    /// Set the pickup policy (`setPickupPolicy`).
    pub fn set_pickup_policy(&self, game: &mut Q2GameServices, policy: Q2PickupPolicy) {
        game.items.pickup_policy = Some(policy);
    }

    /// Set the pickup admission (`setPickupAdmission`).
    pub fn set_pickup_admission(
        &self,
        game: &mut Q2GameServices,
        admission: Option<Box<dyn PickupAdmission>>,
    ) {
        game.items.pickup_admission = admission;
    }

    /// Whether an item maps through the admission (`mapsSupply`).
    pub fn maps_supply(&self, game: &Q2GameServices, item: &str) -> bool {
        game.items.pickup_admission.as_ref().is_some_and(|admission| {
            admission.maps(PickupMapKind::Ammo, &item.to_string())
                || admission.maps(PickupMapKind::Weapons, &item.to_string())
        })
    }

    /// Capture item state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2ItemsCheckpoint {
        let mut pickups = Vec::new();
        let mut powers = Vec::new();
        let mut bindings = Vec::new();
        for (actor, _) in &game.entities {
            let saved = SavedActorId::from(actor);
            if let Some(pickup) = game.items.pickups.get(actor) {
                pickups.push(Q2PickupCheckpoint {
                    actor: saved.clone(),
                    classname: pickup.item.classname.clone(),
                    targets_used: pickup.targets_used,
                    retained: pickup.retained,
                    expires_at: pickup.expires_at,
                });
            }
            if let Some(power) = game.items.powers.get(actor) {
                powers.push(Q2PowerCheckpoint { actor: saved.clone(), state: *power });
            }
            if game.items.power_armor_bindings.contains(actor) {
                bindings.push(saved);
            }
        }
        Q2ItemsCheckpoint {
            power_cube_count: game.items.power_cube_count,
            pickups,
            powers,
            power_armor_bindings: bindings,
        }
    }

    /// Restore item state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2ItemsCheckpoint) {
        game.items.pickups = HashMap::new();
        game.items.powers = HashMap::new();
        game.items.power_armor_bindings = HashSet::new();
        game.items.power_cube_count = checkpoint.power_cube_count;
        for saved in &checkpoint.pickups {
            let owned = restore_q2_actor(game, saved.actor.clone());
            let entity = game.entity(owned.id()).cloned();
            let item = game.items.catalog.get(&saved.classname).cloned();
            match (entity, item) {
                (Some(_), Some(item)) => {
                    game.items.pickups.insert(
                        owned.id().clone(),
                        PickupState {
                            item,
                            targets_used: saved.targets_used,
                            retained: saved.retained,
                            expires_at: saved.expires_at,
                        },
                    );
                }
                _ => panic!("Q2 item checkpoint cannot resolve {}", saved.classname),
            }
        }
        for saved in &checkpoint.powers {
            let owned = restore_q2_actor(game, saved.actor.clone());
            game.items.powers.insert(owned.id().clone(), saved.state);
        }
        for saved in &checkpoint.power_armor_bindings {
            let owned = restore_q2_actor(game, saved.clone());
            bind_power_armor_at(&owned, game);
        }
    }

    /// List inventory items (`list`).
    pub fn list(&self, game: &Q2GameServices) -> Vec<Q2InventoryItem> {
        list_items(game)
    }

    /// Look up an inventory item (`lookup`).
    pub fn lookup(&self, game: &Q2GameServices, value: &str) -> Option<Q2InventoryItem> {
        lookup_item(game, value)
    }

    /// Give starting items (`giveStartItems`).
    pub fn give_start_items(&self, player: &OwnedActor, game: &mut Q2GameServices, expression: &str) {
        give_start_items_at(player, game, expression);
    }

    /// Give an ammo count (`giveAmmoCount`).
    pub fn give_ammo_count(
        &self,
        player: &OwnedActor,
        game: &mut Q2GameServices,
        item_id: &str,
        count: Option<f64>,
    ) {
        give_ammo_count_at(player, game, item_id, count);
    }

    /// Clear powerups (`clearPowerups`).
    pub fn clear_powerups(&self, game: &mut Q2GameServices, player: &ActorId) {
        game.items.powers.remove(player);
    }

    /// Drop an item (`drop`).
    pub fn drop(
        &self,
        this: ActorId,
        game: &mut Q2GameServices,
        item_id: &str,
        options: &Q2DropOptions,
    ) -> Option<ActorId> {
        drop_item_at(this, game, item_id, options)
    }

    /// Drop a monster item (`dropMonster`).
    pub fn drop_monster(
        &self,
        actor: &OwnedActor,
        game: &mut Q2GameServices,
        classname: &str,
    ) -> Option<ActorId> {
        let descriptor = lookup_item(game, classname)?;
        let item = game.items.catalog.get(&descriptor.classname).cloned()?;
        Some(drop_source(actor, game, &item, &Q2DropOptions { player_death: false, ..Q2DropOptions::default() }, 0.0))
    }

    /// Read player powerups (`playerPowerups`).
    pub fn player_powerups(&self, game: &Q2GameServices, player: &ActorId) -> Q2PlayerPowerups {
        game.items.powers.get(player).copied().unwrap_or_default()
    }

    /// Weapon inventory entries (`weaponInventory`).
    pub fn weapon_inventory(&self, game: &Q2GameServices) -> Vec<(ItemId, f64)> {
        weapon_inventory_entries(&game.items.catalog)
    }

    /// Configure a player (`configurePlayer`).
    pub fn configure_player(
        &self,
        actor: &OwnedActor,
        game: &mut Q2GameServices,
        give_blaster: bool,
    ) {
        configure_player_at(actor, game, give_blaster);
    }

    /// Spawn dispatch (`spawn`).
    pub fn spawn(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        spawn_item_entity(actor, game)
    }

    /// Spawn an item (`spawnItem`).
    pub fn spawn_item(
        &self,
        game: &mut Q2GameServices,
        actor: ActorId,
        descriptor: &str,
    ) -> bool {
        spawn_item_at(game, actor, descriptor)
    }

    /// Read a pickup item definition (`itemDefinition`).
    pub fn item_definition(
        &self,
        game: &Q2GameServices,
        actor: &ActorId,
    ) -> Option<Q2ItemDefinition> {
        game.items.pickups.get(actor).map(|pickup| pickup.item.clone())
    }

    /// Preview pickup supply (`previewSupply`).
    pub fn preview_supply(
        &self,
        game: &mut Q2GameServices,
        pickup: ActorId,
        recipient: ActorId,
    ) -> Option<PickupSupplyPreview> {
        preview_supply_at(game, pickup, recipient)
    }

    /// Observe pickup supply (`observeSupply`).
    pub fn observe_supply(
        &self,
        game: &mut Q2GameServices,
        pickup: ActorId,
        recipient: ActorId,
    ) -> Option<PickupSupplyObservation> {
        observe_supply_at(game, pickup, recipient)
    }

    /// Replace a pickup item (`replaceItem`).
    pub fn replace_item(&self, game: &mut Q2GameServices, actor: ActorId, descriptor: &str) {
        replace_item_at(game, actor, descriptor);
    }

    /// Touch a pickup (`touch`).
    pub fn touch(&self, pickup: ActorId, game: &mut Q2GameServices, player: ActorId) {
        touch_item_at(pickup, game, player);
    }

    /// Use an inventory item (`use`).
    pub fn use_inventory_item(
        &self,
        player: &OwnedActor,
        item_id: &str,
        game: &mut Q2GameServices,
        duration: f64,
    ) -> bool {
        use_inventory_item_at(player.clone(), item_id, game, duration)
    }
}

/// Item pickup name (`q2ItemPickupName`).
pub fn q2_item_pickup_name(classname: &str) -> Option<String> {
    base_item_catalog().get(classname).map(|item| item.name.clone())
}

/// Base weapon inventory (`q2BaseWeaponInventory`).
pub fn q2_base_weapon_inventory() -> Vec<(ItemId, f64)> {
    weapon_inventory_entries(&base_item_catalog())
}

/// Base item icons (`q2BaseItemIcons`).
pub fn q2_base_item_icons() -> Vec<(ItemId, String)> {
    let mut icons: Vec<(ItemId, String)> = base_ammunition()
        .iter()
        .map(|item| (item_id(item), item.icon.clone()))
        .collect();
    for weapon in base_weapons() {
        let (_, icon) = base_weapon_names(weapon.name);
        icons.push((weapon.definition.item.clone(), icon.to_string()));
    }
    icons
}

/// Base weapon display name (`q2BaseWeaponDisplayName`).
pub fn q2_base_weapon_display_name(item: &str) -> Option<String> {
    base_weapons()
        .into_iter()
        .find(|weapon| weapon.definition.item == item)
        .map(|weapon| base_weapon_names(weapon.name).0.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armor_item(classname: &str, points: f64, maximum: f64, normal: f64, energy: f64) -> Q2ItemDefinition {
        Q2ItemDefinition {
            classname: classname.to_string(),
            model: String::new(),
            icon: String::new(),
            name: String::new(),
            sound: String::new(),
            rotate: false,
            respawn: 0.0,
            console_give: None,
            kind: Q2ItemKindData::Armor { points, maximum, normal, energy },
        }
    }

    fn shard_item() -> Q2ItemDefinition {
        Q2ItemDefinition {
            classname: "item_armor_shard".to_string(),
            model: String::new(),
            icon: String::new(),
            name: String::new(),
            sound: String::new(),
            rotate: false,
            respawn: 0.0,
            console_give: None,
            kind: Q2ItemKindData::Shard,
        }
    }

    #[test]
    fn builds_base_catalog() {
        let catalog = base_item_catalog();
        assert_eq!(catalog.len(), 44);
        assert!(catalog["weapon_shotgun"].kind() == Q2ItemKind::Weapon);
        assert!(!catalog.contains_key("weapon_grenades"));
        assert_eq!(q2_item_pickup_name("item_quad").as_deref(), Some("Quad Damage"));
        assert_eq!(q2_item_pickup_name("missing"), None);
        assert_eq!(
            q2_base_weapon_display_name("q2:weapon_railgun").as_deref(),
            Some("Railgun")
        );
        assert_eq!(q2_base_item_icons().len(), 17);
    }

    #[test]
    fn picks_up_shards() {
        let none = RegularArmorState::None;
        let fresh = pickup_q2_armor(&shard_item(), &none).expect("shard");
        assert!(matches!(fresh, RegularArmorState::Q2 { points, .. } if points == 2.0));
        let worn = RegularArmorState::Q2 {
            item: "q2:item_armor_jacket".to_string(),
            points: 10.0,
            normal_protection: 0.3,
            energy_protection: 0.0,
        };
        let grown = pickup_q2_armor(&shard_item(), &worn).expect("shard");
        assert!(matches!(grown, RegularArmorState::Q2 { points, .. } if points == 12.0));
    }

    #[test]
    fn picks_up_armor() {
        let none = RegularArmorState::None;
        let jacket = armor_item("item_armor_jacket", 25.0, 50.0, 0.3, 0.0);
        let fresh = pickup_q2_armor(&jacket, &none).expect("fresh");
        assert!(matches!(fresh, RegularArmorState::Q2 { points, .. } if points == 25.0));
        let body = armor_item("item_armor_body", 100.0, 200.0, 0.8, 0.6);
        let worn = RegularArmorState::Q2 {
            item: "q2:item_armor_jacket".to_string(),
            points: 40.0,
            normal_protection: 0.3,
            energy_protection: 0.0,
        };
        let upgraded = pickup_q2_armor(&body, &worn).expect("upgrade");
        assert!(matches!(upgraded, RegularArmorState::Q2 { points, .. } if points == 115.0));
        let downgraded = pickup_q2_armor(&jacket, &upgraded).expect("downgrade");
        assert!(matches!(downgraded, RegularArmorState::Q2 { points, .. } if points == 124.0));
        let source = RegularArmorState::Source { points: 5.0, item: None };
        assert!(pickup_q2_armor(&jacket, &source).is_none());
    }
}
