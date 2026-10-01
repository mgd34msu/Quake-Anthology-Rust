//! Mission-pack items (`src/content/q2/missionpacks/items.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::vec3;

use crate::contract::{InventoryEntry, ItemId};
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2Mode, Q2PresentationEvent, Q2PrintLevel, Q2SpawnFn, SpawnModule,
};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData, Q2ItemModule};
use crate::q2::foundation::weapons::types::Q2WeaponInput;
use crate::q2::foundation::weapons::vectors::angle_vectors;

use super::doppleganger::mission_doppleganger;
use super::projectiles::mission_projectiles;
use super::random_items::{q2_random_item, Q2RandomItemSettings};
use super::spheres::{mission_spheres, Q2SphereKind};
use super::types::{Q2MissionPack, Q2MissionPackPlayerEffect};
use super::weapons::definitions::{rogue_weapon_definitions, xatrix_weapon_definitions};

/// Mission-pack powerup timers (`Q2MissionPackPowerups`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MissionPackPowerups {
    /// Quad-fire expiry.
    pub quad_fire_until: f64,
    /// Double-damage expiry.
    pub double_until: f64,
    /// IR-goggles expiry.
    pub ir_until: f64,
}

impl Q2MissionPackPowerups {
    /// Expired timers.
    fn empty() -> Self {
        Self {
            quad_fire_until: 0.0,
            double_until: 0.0,
            ir_until: 0.0,
        }
    }
}

/// Mission-pack items checkpoint (`Q2MissionPackItemsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MissionPackItemsCheckpoint {
    /// Saved powerup timers.
    pub powers: Vec<(SavedActorId, Q2MissionPackPowerups)>,
}

/// Mission-pack item hooks (`Q2MissionPackItemHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackItemHooks {
    /// Emit a player effect.
    pub player_effect: fn(&Q2GameServices, Q2MissionPackPlayerEffect),
}

/// Disconnected player-effect hook (drops the effect).
fn disconnected_player_effect(_game: &Q2GameServices, _effect: Q2MissionPackPlayerEffect) {}

/// Item hooks used before the session installs its own.
pub fn disconnected_item_hooks() -> Q2MissionPackItemHooks {
    Q2MissionPackItemHooks {
        player_effect: disconnected_player_effect,
    }
}

/// Powerup field selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerField {
    /// Quad fire.
    QuadFire,
    /// Double damage.
    Double,
    /// IR goggles.
    Ir,
}

/// Weapon display names (`names`).
const NAMES: [(&str, &str, &str); 7] = [
    ("ionripper", "Ionripper", "w_ripper"),
    ("phalanx", "Phalanx", "w_phallanx"),
    ("etf_rifle", "ETF Rifle", "w_etf_rifle"),
    ("heatbeam", "Plasma Beam", "w_heatbeam"),
    ("chainfist", "Chainfist", "w_chainfist"),
    ("disintegrator", "Disruptor", "w_disintegrator"),
    ("proxlauncher", "Prox Launcher", "w_proxlaunch"),
];

/// Build an ammo definition.
#[allow(clippy::too_many_arguments)]
fn ammo_definition(
    classname: &str,
    name: &str,
    icon: &str,
    model: &str,
    rotate: bool,
    quantity: f64,
    capacity: f64,
    weapon_ammo: bool,
    infinite_ammo_quantity: Option<Option<f64>>,
) -> Q2ItemDefinition {
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
            infinite_ammo_quantity,
        },
    }
}

/// Build a key definition.
fn key_definition(classname: &str, name: &str, icon: &str, model: &str) -> Q2ItemDefinition {
    Q2ItemDefinition {
        classname: classname.to_string(),
        model: model.to_string(),
        icon: icon.to_string(),
        name: name.to_string(),
        sound: "items/pkup.wav".to_string(),
        rotate: true,
        respawn: 0.0,
        console_give: None,
        kind: Q2ItemKindData::Key,
    }
}

/// Xatrix ammo and keys (`xatrixAmmo`).
fn xatrix_ammo() -> Vec<Q2ItemDefinition> {
    vec![
        ammo_definition(
            "ammo_magslug",
            "Mag Slug",
            "a_mslugs",
            "models/objects/ammo/tris.md2",
            false,
            10.0,
            50.0,
            false,
            None,
        ),
        ammo_definition(
            "ammo_trap",
            "Trap",
            "a_trap",
            "models/weapons/g_trap/tris.md2",
            true,
            1.0,
            5.0,
            true,
            None,
        ),
        key_definition(
            "key_green_key",
            "Green Key",
            "k_green",
            "models/items/keys/green_key/tris.md2",
        ),
    ]
}

/// Rogue ammo and keys (`rogueAmmo`).
fn rogue_ammo() -> Vec<Q2ItemDefinition> {
    vec![
        ammo_definition(
            "ammo_flechettes",
            "Flechettes",
            "a_flechettes",
            "models/ammo/am_flechette/tris.md2",
            false,
            50.0,
            200.0,
            false,
            None,
        ),
        ammo_definition(
            "ammo_prox",
            "Prox",
            "a_prox",
            "models/ammo/am_prox/tris.md2",
            false,
            5.0,
            50.0,
            false,
            None,
        ),
        ammo_definition(
            "ammo_tesla",
            "Tesla",
            "a_tesla",
            "models/ammo/am_tesl/tris.md2",
            false,
            5.0,
            50.0,
            true,
            Some(None),
        ),
        ammo_definition(
            "ammo_disruptor",
            "Rounds",
            "a_disruptor",
            "models/ammo/am_disr/tris.md2",
            false,
            15.0,
            100.0,
            false,
            None,
        ),
        key_definition(
            "key_nuke_container",
            "Antimatter Pod",
            "i_contain",
            "models/weapons/g_nuke/tris.md2",
        ),
        key_definition(
            "key_nuke",
            "Antimatter Bomb",
            "i_nuke",
            "models/weapons/g_nuke/tris.md2",
        ),
    ]
}

/// Mission weapon display name (`q2MissionWeaponDisplayName`).
pub fn q2_mission_weapon_display_name(name: &str) -> Option<String> {
    if let Some(display) = NAMES.iter().find(|entry| entry.0 == name) {
        return Some(display.1.to_string());
    }
    let classname = format!("ammo_{name}");
    xatrix_ammo()
        .into_iter()
        .chain(rogue_ammo())
        .find(|item| item.classname == classname)
        .map(|item| item.name)
}

/// Mission weapon inventory (`q2MissionWeaponInventory`).
pub fn q2_mission_weapon_inventory(pack: Q2MissionPack) -> Vec<(ItemId, f64)> {
    let mut entries = Vec::new();
    let ammo = match pack {
        Q2MissionPack::Xatrix => xatrix_ammo(),
        Q2MissionPack::Rogue => rogue_ammo(),
    };
    for item in ammo {
        if let Q2ItemKindData::Ammo { capacity, .. } = item.kind {
            entries.push((format!("q2:{}", item.classname), capacity));
        }
    }
    let weapons = match pack {
        Q2MissionPack::Xatrix => xatrix_weapon_definitions(),
        Q2MissionPack::Rogue => rogue_weapon_definitions(),
    };
    for weapon in weapons {
        if Some(&weapon.item) != weapon.ammo.as_ref() {
            entries.push((weapon.item, 1.0));
        }
    }
    entries
}

/// Mission weapon icons (`q2MissionWeaponIcons`).
pub fn q2_mission_weapon_icons() -> Vec<(ItemId, String)> {
    let mut entries = Vec::new();
    for item in xatrix_ammo().into_iter().chain(rogue_ammo()) {
        if matches!(item.kind, Q2ItemKindData::Ammo { .. }) {
            entries.push((format!("q2:{}", item.classname), item.icon));
        }
    }
    for weapon in xatrix_weapon_definitions()
        .into_iter()
        .chain(rogue_weapon_definitions())
    {
        if let Some(display) = NAMES.iter().find(|entry| entry.0 == weapon.name) {
            entries.push((weapon.item, display.2.to_string()));
        }
    }
    entries
}

/// Mission-pack items (`Q2MissionPackItems`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackItems {
    /// Item hooks.
    pub hooks: Q2MissionPackItemHooks,
    /// Mission pack.
    pub pack: Q2MissionPack,
    /// Shared item module.
    pub shared_items: Q2ItemModule,
}

impl Q2MissionPackItems {
    /// Spawn a mission item (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        mission_item_spawn_inner(self.pack, &self.shared_items, entity, game)
    }

    /// Substitute a random respawn (`randomRespawn`).
    pub fn random_respawn(
        &self,
        entity: ActorId,
        game: &mut Q2GameServices,
        settings: &Q2RandomItemSettings,
    ) -> Option<ActorId> {
        if game.options.edition == Q2Edition::Classic && self.pack != Q2MissionPack::Rogue {
            return None;
        }
        let item = self.shared_items.item_definition(game, &entity)?;
        let classname = q2_random_item(&entity, &item, game, settings)?;
        if game.options.edition == Q2Edition::Rerelease {
            self.shared_items.replace_item(game, entity.clone(), &classname);
            return Some(entity);
        }
        let replacement = game.create(&classname, BTreeMap::new());
        let body = game.body_of(entity);
        let mut moved = game.body_of(replacement.clone());
        moved.origin = body.origin;
        moved.bounds = body.bounds;
        game.write_body(replacement.clone(), &moved, false);
        game.require_entity_mut(&replacement).gravity_vector = vec3(0.0, 0.0, -1.0);
        if !self.spawn(replacement.clone(), game) {
            panic!("Rogue random item is not registered: {classname}");
        }
        game.require_entity_mut(&replacement).render_flags |= 0x8000;
        Some(replacement)
    }

    /// Register the pack items (`register`).
    pub fn register(&self, game: &mut Q2GameServices, pack: Q2MissionPack, edition: Q2Edition) -> SpawnModule {
        game.mission_packs.item_hooks = self.hooks;
        game.mission_packs.items_pack = Some(pack);
        game.mission_packs.shared_items = Some(self.shared_items);
        let ammo = match pack {
            Q2MissionPack::Xatrix => xatrix_ammo(),
            Q2MissionPack::Rogue => rogue_ammo(),
        };
        for mut item in ammo {
            if item.classname == "ammo_trap" && edition == Q2Edition::Rerelease {
                if let Q2ItemKindData::Ammo {
                    infinite_ammo_quantity, ..
                } = &mut item.kind
                {
                    *infinite_ammo_quantity = Some(None);
                }
            }
            self.shared_items.register_item(game, item);
        }
        let weapons = match pack {
            Q2MissionPack::Xatrix => xatrix_weapon_definitions(),
            Q2MissionPack::Rogue => rogue_weapon_definitions(),
        };
        for definition in weapons {
            let Some(visual) = NAMES.iter().find(|entry| entry.0 == definition.name) else {
                continue;
            };
            self.shared_items.register_item(
                game,
                Q2ItemDefinition {
                    classname: definition.classname,
                    model: definition.world_model,
                    icon: visual.2.to_string(),
                    name: visual.1.to_string(),
                    sound: "misc/w_pkup.wav".to_string(),
                    rotate: true,
                    respawn: 30.0,
                    console_give: None,
                    kind: Q2ItemKindData::Weapon {
                        ammo: definition.ammo,
                        coop_stay: Some(edition == Q2Edition::Rerelease),
                    },
                },
            );
        }
        if pack == Q2MissionPack::Xatrix {
            self.shared_items.register_item(
                game,
                power_definition(
                    "item_quadfire",
                    "DualFire Damage",
                    "p_quadfire",
                    "models/items/quadfire/tris.md2",
                    PowerField::QuadFire,
                    quadfire_pickup,
                    quadfire_use,
                ),
            );
            self.shared_items.register_item(
                game,
                Q2ItemDefinition {
                    classname: "item_foodcube".to_string(),
                    model: "models/objects/trapfx/tris.md2".to_string(),
                    icon: "i_health".to_string(),
                    name: "Health".to_string(),
                    sound: "items/s_health.wav".to_string(),
                    rotate: false,
                    respawn: 0.0,
                    console_give: None,
                    kind: Q2ItemKindData::Custom {
                        capacity: 0.0,
                        quantity: 0.0,
                        coop_stay: false,
                        droppable: false,
                        pickup: foodcube_pickup,
                        use_item: None,
                    },
                },
            );
        } else {
            if edition == Q2Edition::Classic {
                self.shared_items.register_item(
                    game,
                    Q2ItemDefinition {
                        classname: "item_compass".to_string(),
                        model: "models/objects/fire/tris.md2".to_string(),
                        icon: "p_compass".to_string(),
                        name: "compass".to_string(),
                        sound: "items/pkup.wav".to_string(),
                        rotate: true,
                        respawn: 60.0,
                        console_give: None,
                        kind: Q2ItemKindData::Custom {
                            capacity: 32767.0,
                            quantity: 1.0,
                            coop_stay: false,
                            droppable: false,
                            pickup: compass_pickup,
                            use_item: Some(compass_use),
                        },
                    },
                );
            }
            self.shared_items.register_item(
                game,
                Q2ItemDefinition {
                    classname: "item_doppleganger".to_string(),
                    model: "models/items/dopple/tris.md2".to_string(),
                    icon: "p_doppleganger".to_string(),
                    name: "Doppleganger".to_string(),
                    sound: "items/pkup.wav".to_string(),
                    rotate: true,
                    respawn: 90.0,
                    console_give: None,
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 1.0,
                        coop_stay: false,
                        droppable: true,
                        pickup: doppleganger_pickup,
                        use_item: Some(doppleganger_use),
                    },
                },
            );
            self.shared_items.register_item(
                game,
                power_definition(
                    "item_double",
                    "Double Damage",
                    "p_double",
                    "models/items/ddamage/tris.md2",
                    PowerField::Double,
                    double_pickup,
                    double_use,
                ),
            );
            self.shared_items.register_item(
                game,
                power_definition(
                    "item_ir_goggles",
                    "IR Goggles",
                    "p_ir",
                    "models/items/goggles/tris.md2",
                    PowerField::Ir,
                    ir_pickup,
                    ir_use,
                ),
            );
            self.shared_items.register_item(
                game,
                Q2ItemDefinition {
                    classname: "ammo_nuke".to_string(),
                    model: "models/weapons/g_nuke/tris.md2".to_string(),
                    icon: "p_nuke".to_string(),
                    name: "A-M Bomb".to_string(),
                    sound: "misc/am_pkup.wav".to_string(),
                    rotate: true,
                    respawn: 300.0,
                    console_give: None,
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 1.0,
                        coop_stay: false,
                        droppable: true,
                        pickup: nuke_pickup,
                        use_item: Some(nuke_use),
                    },
                },
            );
            self.shared_items
                .register_item(game, sphere_definition(Q2SphereKind::Defender));
            self.shared_items
                .register_item(game, sphere_definition(Q2SphereKind::Hunter));
            self.shared_items
                .register_item(game, sphere_definition(Q2SphereKind::Vengeance));
        }
        SpawnModule {
            spawn: mission_item_spawn as Q2SpawnFn,
            item_name: |_| None,
            callbacks: crate::q2::foundation::callbacks::Q2CallbackDefinitions::default(),
        }
    }

    /// Read powerup timers (`powerups`).
    pub fn powerups(&self, actor: &ActorId, game: &Q2GameServices) -> Q2MissionPackPowerups {
        let _ = self;
        game.mission_packs
            .item_powers
            .get(actor)
            .copied()
            .unwrap_or_else(Q2MissionPackPowerups::empty)
    }

    /// Fold powerup timers into weapon input (`input`).
    pub fn input(&self, actor: &ActorId, game: &Q2GameServices, mut input: Q2WeaponInput) -> Q2WeaponInput {
        let powers = self.powerups(actor, game);
        input.quad_fire_until = input.quad_fire_until.max(powers.quad_fire_until);
        input.double_until = input.double_until.max(powers.double_until);
        input
    }

    /// Clear powerup timers (`reset`).
    pub fn reset(&self, actor: &ActorId, game: &mut Q2GameServices) {
        let _ = self;
        game.mission_packs.item_powers.remove(actor);
    }

    /// Grant pack ammo (`ammoPack`).
    pub fn ammo_pack(&self, player: &OwnedActor, game: &mut Q2GameServices, full: bool, pack: Q2MissionPack) {
        let _ = self;
        let changes: &[(&str, f64, f64)] = match pack {
            Q2MissionPack::Xatrix => &[(
                "q2:ammo_magslug",
                if full { 100.0 } else { 75.0 },
                if full { 10.0 } else { 0.0 },
            )],
            Q2MissionPack::Rogue => &[
                (
                    "q2:ammo_flechettes",
                    if full { 200.0 } else { 250.0 },
                    if full { 50.0 } else { 0.0 },
                ),
                (
                    "q2:ammo_disruptor",
                    if full { 200.0 } else { 150.0 },
                    if full { 15.0 } else { 0.0 },
                ),
            ],
        };
        for (item, capacity, give) in changes {
            let current = game
                .host
                .inventory()
                .entries(player.id())
                .into_iter()
                .find(|entry| entry.item == *item);
            game.host.inventory().configure(
                player,
                &InventoryEntry {
                    item: item.to_string(),
                    count: current.as_ref().map(|entry| entry.count).unwrap_or(0.0),
                    capacity: current
                        .as_ref()
                        .map(|entry| entry.capacity)
                        .unwrap_or(0.0)
                        .max(*capacity),
                    count_policy: None,
                },
            );
            game.host.inventory().give(player, &item.to_string(), *give);
        }
    }

    /// Capture powerup timers (`capture`).
    pub fn capture(&self, game: &mut Q2GameServices) -> Q2MissionPackItemsCheckpoint {
        let _ = self;
        let mut powers: Vec<(SavedActorId, Q2MissionPackPowerups)> = game
            .mission_packs
            .item_powers
            .iter()
            .filter(|(actor, _)| game.host.actors().is_live(actor))
            .filter_map(|(actor, state)| save_q2_actor(Some(actor)).map(|saved| (saved, *state)))
            .collect();
        powers.sort_by_key(|left| (left.0.slot, left.0.generation));
        Q2MissionPackItemsCheckpoint { powers }
    }

    /// Restore powerup timers (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2MissionPackItemsCheckpoint) {
        let _ = self;
        game.mission_packs.item_powers = checkpoint
            .powers
            .iter()
            .map(|(saved, state)| (restore_q2_actor(game, *saved).id().clone(), *state))
            .collect();
    }
}

/// Spawn entry for the module table.
pub(crate) fn mission_item_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let pack = game
        .mission_packs
        .items_pack
        .expect("Q2 mission-pack items are not registered");
    let shared = game
        .mission_packs
        .shared_items
        .expect("Q2 mission-pack items are not registered");
    mission_item_spawn_inner(pack, &shared, entity, game)
}

/// Spawn a mission item with explicit module state (`spawn`).
fn mission_item_spawn_inner(
    pack: Q2MissionPack,
    shared_items: &Q2ItemModule,
    entity: ActorId,
    game: &mut Q2GameServices,
) -> bool {
    if pack == Q2MissionPack::Rogue || game.options.edition == Q2Edition::Rerelease {
        let classname = game.require_entity(&entity).classname.clone();
        if classname == "weapon_nailgun" {
            game.require_entity_mut(&entity).classname = "weapon_etf_rifle".to_string();
        } else if classname == "ammo_nails" {
            game.require_entity_mut(&entity).classname = "ammo_flechettes".to_string();
        } else if classname == "weapon_heatbeam" {
            game.require_entity_mut(&entity).classname = "weapon_plasmabeam".to_string();
        }
    }
    let classname = game.require_entity(&entity).classname.clone();
    let classic_rogue = pack == Q2MissionPack::Rogue && game.options.edition == Q2Edition::Classic;
    let mut descriptor = classname.clone();
    if classic_rogue {
        if classname == "ammo_magslug" {
            descriptor = "ammo_flechettes".to_string();
        } else if classname == "ammo_trap" {
            descriptor = "weapon_proxlauncher".to_string();
        } else if classname == "weapon_boomer" {
            descriptor = "weapon_etf_rifle".to_string();
        } else if classname == "weapon_phalanx" {
            descriptor = "weapon_plasmabeam".to_string();
        } else if classname == "item_quadfire" {
            let chance = game.host.random();
            descriptor = if chance < 0.2 {
                "item_sphere_hunter"
            } else if chance < 0.6 {
                "item_sphere_vengeance"
            } else {
                "item_sphere_defender"
            }
            .to_string();
        }
    }
    if shared_items.item_name(game, &descriptor).is_none() {
        return false;
    }
    let deathmatch = game.options.mode == Q2Mode::Deathmatch;
    let flags = game.options.deathmatch_flags;
    let sphere = descriptor.starts_with("item_sphere_");
    let power = descriptor == "item_quadfire"
        || descriptor == "item_double"
        || descriptor == "item_ir_goggles"
        || descriptor == "item_compass" && classic_rogue;
    if deathmatch
        && (flags & 1 != 0 && classname == "item_foodcube"
            || flags & 2 != 0 && (power || sphere || classname == "item_doppleganger")
            || pack == Q2MissionPack::Rogue
                && (flags & 0x20000 != 0 && (classname == "ammo_prox" || classname == "ammo_tesla")
                    || flags & 0x80000 != 0 && classname == "ammo_nuke"
                    || flags & 0x100000 != 0 && sphere))
        || classic_rogue
            && (classname == "ammo_disruptor"
                || classname == "weapon_disintegrator"
                || !deathmatch
                    && (descriptor == "ammo_nuke"
                        || descriptor == "item_doppleganger"
                        || descriptor == "item_sphere_hunter"
                        || descriptor == "item_sphere_vengeance"))
    {
        game.remove_actor(entity);
        return true;
    }
    if classic_rogue && game.require_entity(&entity).spawnflags > 1 && classname != "key_power_cube" {
        game.require_entity_mut(&entity).spawnflags = 0;
    }
    if !shared_items.spawn_item(game, entity.clone(), &descriptor) {
        return false;
    }
    if classname == "item_foodcube" {
        game.require_entity_mut(&entity).spawnflags |= 0x10000;
        game.require_entity_mut(&entity).classname = "foodcube".to_string();
    }
    true
}

/// Build a powerup definition (`power`).
fn power_definition(
    classname: &str,
    name: &str,
    icon: &str,
    model: &str,
    _field: PowerField,
    pickup: crate::q2::foundation::items::Q2CustomPickup,
    use_item: crate::q2::foundation::items::Q2CustomUse,
) -> Q2ItemDefinition {
    Q2ItemDefinition {
        classname: classname.to_string(),
        model: model.to_string(),
        icon: icon.to_string(),
        name: name.to_string(),
        sound: "items/pkup.wav".to_string(),
        rotate: true,
        respawn: 60.0,
        console_give: None,
        kind: Q2ItemKindData::Custom {
            capacity: 32767.0,
            quantity: 1.0,
            coop_stay: false,
            droppable: true,
            pickup,
            use_item: Some(use_item),
        },
    }
}

/// Use a powerup (`power.use`).
fn power_use(
    player: &OwnedActor,
    game: &mut Q2GameServices,
    item: &str,
    field: PowerField,
    sound: &str,
    timeout: f64,
) -> bool {
    if !game.host.inventory().consume(player, &item.to_string(), 1.0) {
        return false;
    }
    let mut powers = game
        .mission_packs
        .item_powers
        .get(player.id())
        .copied()
        .unwrap_or_else(Q2MissionPackPowerups::empty);
    let slot = match field {
        PowerField::QuadFire => &mut powers.quad_fire_until,
        PowerField::Double => &mut powers.double_until,
        PowerField::Ir => &mut powers.ir_until,
    };
    *slot = game.host.now().max(*slot) + timeout;
    game.mission_packs.item_powers.insert(player.id().clone(), powers);
    if game.entity(player.id()).is_some() {
        game.sound(player.id(), sound, 3, 1.0, 1.0);
    }
    if field == PowerField::Ir {
        (game.mission_packs.item_hooks.player_effect)(
            game,
            Q2MissionPackPlayerEffect::Ir {
                actor: player.id().clone(),
                until: powers.ir_until,
            },
        );
    }
    true
}

/// Pick up a powerup (`power.pickup`).
fn power_pickup(
    entity: &ActorId,
    game: &mut Q2GameServices,
    player: &OwnedActor,
    item: &str,
    field: PowerField,
    duration: f64,
) -> bool {
    let count = game.host.inventory().count(player.id(), &item.to_string());
    if game.options.skill == 1 && count >= 2.0 || game.options.skill >= 2 && count >= 1.0 {
        return false;
    }
    if game.host.inventory().give(player, &item.to_string(), 1.0) == 0.0 {
        return false;
    }
    let dropped_quad_fire = field == PowerField::QuadFire && game.require_entity(entity).spawnflags & 0x20000 != 0;
    if game.options.deathmatch_flags & 16 != 0 || dropped_quad_fire {
        let timeout = if dropped_quad_fire {
            game.require_entity(entity)
                .next_think
                .map(|next| 0.0f64.max(next - game.host.now()))
                .unwrap_or(duration)
        } else {
            duration
        };
        let sound = match field {
            PowerField::QuadFire => "items/quadfire1.wav",
            PowerField::Double => "misc/ddamage1.wav",
            PowerField::Ir => "misc/ir_start.wav",
        };
        power_use(player, game, item, field, sound, timeout);
    }
    true
}

/// Pick up quad fire.
fn quadfire_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    power_pickup(&entity, game, &player, "q2:item_quadfire", PowerField::QuadFire, 30.0)
}

/// Use quad fire.
fn quadfire_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    power_use(
        &player,
        game,
        "q2:item_quadfire",
        PowerField::QuadFire,
        "items/quadfire1.wav",
        30.0,
    )
}

/// Pick up double damage.
fn double_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    power_pickup(&entity, game, &player, "q2:item_double", PowerField::Double, 30.0)
}

/// Use double damage.
fn double_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    power_use(
        &player,
        game,
        "q2:item_double",
        PowerField::Double,
        "misc/ddamage1.wav",
        30.0,
    )
}

/// Pick up IR goggles.
fn ir_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    power_pickup(&entity, game, &player, "q2:item_ir_goggles", PowerField::Ir, 60.0)
}

/// Use IR goggles.
fn ir_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    power_use(
        &player,
        game,
        "q2:item_ir_goggles",
        PowerField::Ir,
        "misc/ir_start.wav",
        60.0,
    )
}

/// Pick up a food cube.
fn foodcube_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let Some(health) = game.host.combat().read(player.id()).map(|combat| combat.health) else {
        return false;
    };
    let count = game.require_entity(&entity).count;
    game.host.combat().set_health(&player, health + f64::from(count));
    true
}

/// Pick up the compass.
fn compass_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let count = game.host.inventory().count(player.id(), &"q2:item_compass".to_string());
    !(game.options.skill == 1 && count >= 2.0 || game.options.skill >= 2 && count >= 1.0)
        && game.host.inventory().give(&player, &"q2:item_compass".to_string(), 1.0) > 0.0
}

/// Use the compass.
fn compass_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    let Some(body) = game.host.bodies().read(player.id()) else {
        return false;
    };
    let mut yaw = game
        .weapons
        .inputs
        .get(player.id())
        .map(|input| input.angles.y)
        .unwrap_or(body.angles.y)
        .trunc() as i32;
    if yaw < 0 {
        yaw += 360;
    }
    game.host.emit(Q2PresentationEvent::Print {
        actor: Some(player.id().clone()),
        level: Q2PrintLevel::High,
        text: format!(
            "Origin: {:.0},{:.0},{:.0}    Dir: {yaw}\n",
            body.origin.x, body.origin.y, body.origin.z
        ),
    });
    true
}

/// Pick up the doppleganger.
fn doppleganger_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    game.options.mode == Q2Mode::Deathmatch
        && game
            .host
            .inventory()
            .give(&player, &"q2:item_doppleganger".to_string(), 1.0)
            > 0.0
}

/// Use the doppleganger.
fn doppleganger_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    game.entity(player.id()).is_some() && mission_doppleganger(game).use_doppleganger(player.id(), game)
}

/// Pick up the antimatter bomb.
fn nuke_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    game.host.inventory().give(&player, &"q2:ammo_nuke".to_string(), 1.0) > 0.0
}

/// Use the antimatter bomb.
fn nuke_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    let owner = match game.entity(player.id()) {
        Some(entity) => entity.actor.id().clone(),
        None => return false,
    };
    if !game.host.inventory().consume(&player, &"q2:ammo_nuke".to_string(), 1.0) {
        return false;
    }
    let input = game.weapons.inputs.get(player.id()).cloned();
    let powers = game
        .mission_packs
        .item_powers
        .get(player.id())
        .copied()
        .unwrap_or_else(Q2MissionPackPowerups::empty);
    let quad = input.as_ref().map(|input| input.quad_until).unwrap_or(0.0) > game.host.now();
    let double = powers
        .double_until
        .max(input.as_ref().map(|input| input.double_until).unwrap_or(0.0))
        > game.host.now()
        && !(quad && input.as_ref().map(|input| input.no_stack_double).unwrap_or(false));
    let multiplier = (if quad { 4.0 } else { 1.0 }) * (if double { 2.0 } else { 1.0 });
    let body = game.body_of(owner.clone());
    let angles = input.map(|input| input.angles).unwrap_or(body.angles);
    mission_projectiles(game).fire_nuke(
        owner,
        game,
        body.origin,
        angle_vectors(angles).forward,
        100.0,
        multiplier,
    );
    true
}

/// Build a sphere definition.
fn sphere_definition(kind: Q2SphereKind) -> Q2ItemDefinition {
    let (classname, name, model, pickup, use_item, respawn) = match kind {
        Q2SphereKind::Defender => (
            "item_sphere_defender",
            "defender sphere",
            "models/items/defender/tris.md2",
            defender_pickup as crate::q2::foundation::items::Q2CustomPickup,
            defender_use as crate::q2::foundation::items::Q2CustomUse,
            60.0,
        ),
        Q2SphereKind::Hunter => (
            "item_sphere_hunter",
            "hunter sphere",
            "models/items/hunter/tris.md2",
            hunter_pickup as crate::q2::foundation::items::Q2CustomPickup,
            hunter_use as crate::q2::foundation::items::Q2CustomUse,
            120.0,
        ),
        Q2SphereKind::Vengeance => (
            "item_sphere_vengeance",
            "vengeance sphere",
            "models/items/vengnce/tris.md2",
            vengeance_pickup as crate::q2::foundation::items::Q2CustomPickup,
            vengeance_use as crate::q2::foundation::items::Q2CustomUse,
            60.0,
        ),
    };
    Q2ItemDefinition {
        classname: classname.to_string(),
        model: model.to_string(),
        icon: format!(
            "p_{}",
            match kind {
                Q2SphereKind::Defender => "defender",
                Q2SphereKind::Hunter => "hunter",
                Q2SphereKind::Vengeance => "vengeance",
            }
        ),
        name: name.to_string(),
        sound: "items/pkup.wav".to_string(),
        rotate: true,
        respawn,
        console_give: None,
        kind: Q2ItemKindData::Custom {
            capacity: 32767.0,
            quantity: 1.0,
            coop_stay: false,
            droppable: false,
            pickup,
            use_item: Some(use_item),
        },
    }
}

/// Use a sphere (`use`).
fn sphere_use(player: &OwnedActor, game: &mut Q2GameServices, id: &str, kind: Q2SphereKind) -> bool {
    let owner = match game.entity(player.id()) {
        Some(entity) => entity.actor.id().clone(),
        None => return false,
    };
    if mission_spheres(game).owned_sphere(player.id(), game).is_some()
        || !game.host.inventory().consume(player, &id.to_string(), 1.0)
    {
        return false;
    }
    mission_spheres(game).launch(&owner, game, kind, false);
    true
}

/// Pick up a sphere (`pickup`).
fn sphere_pickup(game: &mut Q2GameServices, player: &OwnedActor, id: &str, kind: Q2SphereKind) -> bool {
    let count = game.host.inventory().count(player.id(), &id.to_string());
    if mission_spheres(game).owned_sphere(player.id(), game).is_some()
        || game.options.skill == 1 && count >= 2.0
        || game.options.skill >= 2 && count >= 1.0
    {
        return false;
    }
    if game.host.inventory().give(player, &id.to_string(), 1.0) == 0.0 {
        return false;
    }
    if game.options.mode == Q2Mode::Deathmatch && game.options.deathmatch_flags & 16 != 0 {
        sphere_use(player, game, id, kind);
    }
    true
}

/// Pick up a defender sphere.
fn defender_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    sphere_pickup(game, &player, "q2:item_sphere_defender", Q2SphereKind::Defender)
}

/// Use a defender sphere.
fn defender_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    sphere_use(&player, game, "q2:item_sphere_defender", Q2SphereKind::Defender)
}

/// Pick up a hunter sphere.
fn hunter_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    sphere_pickup(game, &player, "q2:item_sphere_hunter", Q2SphereKind::Hunter)
}

/// Use a hunter sphere.
fn hunter_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    sphere_use(&player, game, "q2:item_sphere_hunter", Q2SphereKind::Hunter)
}

/// Pick up a vengeance sphere.
fn vengeance_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    sphere_pickup(game, &player, "q2:item_sphere_vengeance", Q2SphereKind::Vengeance)
}

/// Use a vengeance sphere.
fn vengeance_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    sphere_use(&player, game, "q2:item_sphere_vengeance", Q2SphereKind::Vengeance)
}
